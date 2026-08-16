#[cfg(not(feature = "wasm"))]
fn main() {}

#[cfg(feature = "wasm")]
mod plugin {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use zellij_ai_session_core::{
        AiSession, CommandSpec, IndexSnapshot, ProjectSummary, RuntimeConfidence, RuntimeRef,
        SessionStatus, search_key,
    };
    use zellij_tile::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    enum View {
        #[default]
        Projects,
        Sessions,
        Search,
        NewSession,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    enum OpenMode {
        #[default]
        Tab,
        Pane,
    }

    #[derive(Default)]
    pub struct AiSessionPlugin {
        snapshot: Option<IndexSnapshot>,
        view: View,
        selected: usize,
        project_id: Option<String>,
        search_query: String,
        status: String,
        indexer: String,
        open_mode: OpenMode,
        scroll_offset: usize,
    }

    impl ZellijPlugin for AiSessionPlugin {
        fn load(&mut self, configuration: BTreeMap<String, String>) {
            self.indexer = configuration
                .get("indexer")
                .cloned()
                .unwrap_or_else(|| "zellij-ai-session-index".into());
            self.open_mode = match configuration.get("open_mode").map(String::as_str) {
                Some("pane") => OpenMode::Pane,
                _ => OpenMode::Tab,
            };
            self.status = "Loading sessions…".into();
            subscribe(&[
                EventType::Key,
                EventType::PaneUpdate,
                EventType::RunCommandResult,
                EventType::CommandPaneOpened,
                EventType::CommandPaneExited,
                EventType::PermissionRequestResult,
                EventType::PastedText,
                EventType::Visible,
                EventType::Timer,
            ]);
            request_permission(&[
                PermissionType::ReadApplicationState,
                PermissionType::RunCommands,
                PermissionType::ChangeApplicationState,
            ]);
            set_timeout(0.2);
            self.refresh();
        }

        fn update(&mut self, event: Event) -> bool {
            match event {
                Event::Key(key) => self.handle_key(key),
                Event::PaneUpdate(manifest) => {
                    self.update_runtime(manifest);
                    true
                }
                Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                    self.handle_command_result(exit_code, stdout, stderr, context)
                }
                Event::PastedText(text) => self.handle_pasted_text(text),
                Event::CommandPaneExited(_, _, _) => {
                    self.refresh();
                    true
                }
                Event::PermissionRequestResult(_) | Event::Visible(true) => {
                    self.refresh();
                    true
                }
                Event::Timer(_) => {
                    self.refresh();
                    true
                }
                _ => false,
            }
        }

        fn render(&mut self, rows: usize, cols: usize) {
            print!("\x1b[2J\x1b[H");
            println!("AI Sessions{}", " ".repeat(cols.saturating_sub(12)));
            println!();

            let viewport = rows.saturating_sub(7).max(1);
            self.ensure_visible(viewport);
            match self.view {
                View::Projects => self.render_projects(viewport),
                View::Sessions => self.render_sessions(viewport),
                View::Search => self.render_search(viewport),
                View::NewSession => self.render_new_session(viewport),
            }

            println!();
            println!("{}", "─".repeat(cols.max(1)));
            match self.view {
            View::Projects => println!("Enter open   / search   r refresh   q close"),
            View::Sessions => println!("Enter open   n new   x close runtime   Esc back   / search   r refresh"),
            View::Search => println!("Type or paste to search   Backspace erase   Esc back"),
            View::NewSession => println!("Enter create   Esc back"),
            }
            if !self.status.is_empty() {
                println!("{}", self.status);
            }
        }
    }

    impl AiSessionPlugin {
        fn refresh(&mut self) {
            let mut context = BTreeMap::new();
            context.insert("action".into(), "index".into());
            run_command(&[self.indexer.as_str()], context);
        }

        fn update_runtime(&mut self, manifest: PaneManifest) {
            let Some(snapshot) = &mut self.snapshot else {
                return;
            };
            let mut runtimes = Vec::new();
            for (tab_position, panes) in manifest.panes {
                for pane in panes {
                    if pane.is_plugin || pane.exited || !pane.is_selectable {
                        continue;
                    }
                    let Some(command) = pane.terminal_command else {
                        continue;
                    };
                    let agent = if command.contains("codex") {
                        Some("codex")
                    } else if command.contains("opencode") {
                        Some("opencode")
                    } else {
                        None
                    };
                    let Some(agent) = agent else {
                        continue;
                    };
                    runtimes.push(RuntimeRef {
                        zellij_session: None,
                        tab_id: Some(tab_position as u32),
                        pane_id: Some(pane.id),
                        cwd: get_pane_cwd(PaneId::Terminal(pane.id)).ok(),
                        command: Some(format!("{agent} {command}")),
                        confidence: RuntimeConfidence::Heuristic,
                    });
                }
            }
            for session in &mut snapshot.sessions {
                let candidates: Vec<&RuntimeRef> = runtimes
                    .iter()
                    .filter(|runtime| {
                        runtime.cwd.as_deref() == Some(session.directory.as_path())
                            && runtime.command.as_deref().is_some_and(|command| {
                                command.contains(session.agent.command_name())
                            })
                    })
                    .collect();
                let exact = candidates.iter().find(|runtime| {
                    runtime
                        .command
                        .as_deref()
                        .is_some_and(|command| command.contains(&session.agent_session_id))
                });
                let runtime = exact
                    .or_else(|| candidates.first().filter(|_| candidates.len() == 1))
                    .copied();
                session.runtime = runtime.cloned();
                session.status = if session.runtime.is_some() {
                    SessionStatus::Running
                } else {
                    SessionStatus::Historical
                };
            }
            self.clamp_selection();
        }

        fn handle_command_result(
            &mut self,
            exit_code: Option<i32>,
            stdout: Vec<u8>,
            stderr: Vec<u8>,
            context: BTreeMap<String, String>,
        ) -> bool {
            if matches!(context.get("action").map(String::as_str), Some("resume" | "new")) {
                if exit_code == Some(0) {
                    match serde_json::from_slice::<CommandSpec>(&stdout) {
                        Ok(command) => self.open_command(command),
                        Err(error) => self.status = format!("Invalid resume command: {error}"),
                    }
                } else {
                    self.status = command_error(stderr);
                }
                return true;
            }

            if exit_code == Some(0) {
                match serde_json::from_slice::<IndexSnapshot>(&stdout) {
                    Ok(snapshot) => {
                        self.snapshot = Some(snapshot);
                        self.status.clear();
                        self.clamp_selection();
                    }
                    Err(error) => self.status = format!("Invalid index snapshot: {error}"),
                }
            } else {
                self.status = command_error(stderr);
            }
            true
        }

        fn open_command(&mut self, command: CommandSpec) {
            let command = CommandToRun {
                path: PathBuf::from(command.program),
                args: command.args,
                cwd: Some(command.cwd),
            };
            let mut context = BTreeMap::new();
            context.insert("source".into(), "zellij-ai-session".into());
            let opened = match self.open_mode {
                OpenMode::Pane => open_command_pane_near_plugin(command, context).is_some(),
                OpenMode::Tab => {
                    let (tab_id, pane_id) = open_command_pane_in_new_tab(command, context);
                    tab_id.is_some() || pane_id.is_some()
                }
            };
            if !opened {
                self.status = "Unable to open Zellij command pane/tab".into();
            }
        }

        fn handle_key(&mut self, key: KeyWithModifier) -> bool {
            if matches!(self.view, View::Search) {
                return self.handle_search_key(key.bare_key);
            }
            if !key.has_no_modifiers() {
                return false;
            }
            match self.view {
                View::Search => false,
                View::Projects => match key.bare_key {
                    BareKey::Down | BareKey::Char('j') => {
                        self.move_selection(1);
                        true
                    }
                    BareKey::Up | BareKey::Char('k') => {
                        self.move_selection(-1);
                        true
                    }
                    BareKey::Enter => {
                        self.open_project();
                        true
                    }
                    BareKey::Char('/') => {
                        self.view = View::Search;
                        self.selected = 0;
                        self.scroll_offset = 0;
                        true
                    }
                    BareKey::Char('r') => {
                        self.refresh();
                        true
                    }
                    BareKey::Char('q') => {
                        close_focus();
                        false
                    }
                    _ => false,
                },
                View::Sessions => match key.bare_key {
                    BareKey::Down | BareKey::Char('j') => {
                        self.move_selection(1);
                        true
                    }
                    BareKey::Up | BareKey::Char('k') => {
                        self.move_selection(-1);
                        true
                    }
                    BareKey::Enter => {
                        self.open_selected_session();
                        true
                    }
                    BareKey::Char('n') => {
                        self.view = View::NewSession;
                        self.selected = 0;
                        self.scroll_offset = 0;
                        true
                    }
                    BareKey::Char('x') => {
                        self.close_selected_runtime();
                        true
                    }
                    BareKey::Esc => {
                        self.view = View::Projects;
                        self.selected = 0;
                        self.scroll_offset = 0;
                        true
                    }
                    BareKey::Char('/') => {
                        self.view = View::Search;
                        self.selected = 0;
                        self.scroll_offset = 0;
                        true
                    }
                    BareKey::Char('r') => {
                        self.refresh();
                        true
                    }
                    _ => false,
                },
                View::NewSession => match key.bare_key {
                    BareKey::Down | BareKey::Char('j') => {
                        self.move_selection(1);
                        true
                    }
                    BareKey::Up | BareKey::Char('k') => {
                        self.move_selection(-1);
                        true
                    }
                    BareKey::Enter => {
                        self.create_selected_session();
                        true
                    }
                    BareKey::Esc => {
                        self.view = View::Sessions;
                        self.selected = 0;
                        self.scroll_offset = 0;
                        true
                    }
                    _ => false,
                },
            }
        }

        fn handle_search_key(&mut self, key: BareKey) -> bool {
            match key {
                BareKey::Esc => {
                    self.view = View::Projects;
                    self.selected = 0;
                    self.scroll_offset = 0;
                    true
                }
                BareKey::Backspace => {
                    self.search_query.pop();
                    self.selected = 0;
                    self.scroll_offset = 0;
                    true
                }
                BareKey::Char(character) => {
                    self.search_query.push(character);
                    self.selected = 0;
                    self.scroll_offset = 0;
                    true
                }
                BareKey::Down => {
                    self.move_selection(1);
                    true
                }
                BareKey::Up => {
                    self.move_selection(-1);
                    true
                }
                BareKey::Enter => {
                    self.open_selected_search_session();
                    true
                }
                _ => false,
            }
        }

        fn handle_pasted_text(&mut self, text: String) -> bool {
            if !matches!(self.view, View::Search) {
                return false;
            }
            self.search_query
                .extend(text.chars().filter(|character| !matches!(character, '\n' | '\r')));
            self.selected = 0;
            self.scroll_offset = 0;
            true
        }

        fn open_project(&mut self) {
            let projects = self.projects();
            let Some(project) = projects.get(self.selected) else {
                return;
            };
            self.project_id = Some(project.project.id.clone());
            self.view = View::Sessions;
            self.selected = 0;
            self.scroll_offset = 0;
        }

        fn create_selected_session(&mut self) {
            let Some(project_id) = self.project_id.clone() else {
                self.status = "Select a project first".into();
                return;
            };
            let Some(snapshot) = &self.snapshot else {
                return;
            };
            let Some(project) = snapshot
                .projects
                .iter()
                .find(|project| project.project.id == project_id)
            else {
                self.status = "Project not found".into();
                return;
            };
            let agent = match self.selected {
                0 => "codex",
                1 => "opencode",
                _ => return,
            };
            let mut context = BTreeMap::new();
            context.insert("action".into(), "new".into());
            let cwd = project.project.root_directory.to_string_lossy().to_string();
            run_command(
                &[
                    self.indexer.as_str(),
                    "new",
                    "--agent",
                    agent,
                    "--cwd",
                    cwd.as_str(),
                ],
                context,
            );
            self.status = format!("Starting {agent}…");
            self.view = View::Sessions;
            self.selected = 0;
            self.scroll_offset = 0;
        }

        fn close_selected_runtime(&mut self) {
            let Some(session) = self
                .sessions_for_current_project()
                .get(self.selected)
                .cloned()
            else {
                return;
            };
            let Some(pane_id) = session.runtime.and_then(|runtime| runtime.pane_id) else {
                self.status = "This Session has no running runtime".into();
                return;
            };
            close_pane_with_id(PaneId::Terminal(pane_id));
            self.status = format!("Closed runtime: {}", session.title);
            self.refresh();
        }

        fn open_selected_session(&mut self) {
            let Some(session) = self
                .sessions_for_current_project()
                .get(self.selected)
                .cloned()
            else {
                return;
            };
            self.resume_or_open(session);
        }

        fn open_selected_search_session(&mut self) {
            let Some(session) = self.search_results().get(self.selected).cloned() else {
                return;
            };
            self.resume_or_open(session);
        }

        fn resume_or_open(&mut self, session: AiSession) {
            if let Some(runtime) = session.runtime.and_then(|runtime| runtime.pane_id) {
                show_pane_with_id(PaneId::Terminal(runtime), true, true);
                return;
            }
            let mut context = BTreeMap::new();
            context.insert("action".into(), "resume".into());
            run_command(
                &[
                    self.indexer.as_str(),
                    "resume",
                    "--agent",
                    session.agent.command_name(),
                    "--session-id",
                    session.agent_session_id.as_str(),
                    "--cwd",
                    session.directory.to_string_lossy().as_ref(),
                ],
                context,
            );
            self.status = format!("Resuming {}…", session.title);
        }

        fn projects(&self) -> Vec<ProjectSummary> {
            self.snapshot
                .as_ref()
                .map(|snapshot| snapshot.projects.clone())
                .unwrap_or_default()
        }

        fn sessions_for_current_project(&self) -> Vec<AiSession> {
            let Some(snapshot) = &self.snapshot else {
                return Vec::new();
            };
            let Some(project_id) = &self.project_id else {
                return Vec::new();
            };
            snapshot
                .sessions
                .iter()
                .filter(|session| &session.project_id == project_id)
                .cloned()
                .collect()
        }

        fn search_results(&self) -> Vec<AiSession> {
            let Some(snapshot) = &self.snapshot else {
                return Vec::new();
            };
            let query = search_key(self.search_query.trim());
            snapshot
                .sessions
                .iter()
                .filter(|session| {
                    let project = snapshot
                        .projects
                        .iter()
                        .find(|project| project.project.id == session.project_id);
                    let project_name = project
                        .map(|project| project.project.name.as_str())
                        .unwrap_or_default();
                    query.is_empty()
                        || search_key(&session.title).contains(&query)
                        || session.agent.command_name().contains(&query)
                        || search_key(&session.directory.to_string_lossy()).contains(&query)
                        || search_key(project_name).contains(&query)
                })
                .cloned()
                .collect()
        }

        fn move_selection(&mut self, delta: isize) {
            let len = self.list_len();
            if len == 0 {
                self.selected = 0;
                return;
            }
            self.selected = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        }

        fn clamp_selection(&mut self) {
            let len = self.list_len();
            self.selected = self.selected.min(len.saturating_sub(1));
            if len == 0 {
                self.scroll_offset = 0;
            } else {
                self.scroll_offset = self.scroll_offset.min(len - 1);
            }
        }

        fn list_len(&self) -> usize {
            match self.view {
                View::Projects => self.projects().len(),
                View::Sessions => self.sessions_for_current_project().len(),
                View::Search => self.search_results().len(),
                View::NewSession => 2,
            }
        }

        fn ensure_visible(&mut self, viewport: usize) {
            let len = self.list_len();
            if len == 0 {
                self.selected = 0;
                self.scroll_offset = 0;
                return;
            }

            self.selected = self.selected.min(len - 1);
            if self.selected < self.scroll_offset {
                self.scroll_offset = self.selected;
            } else if self.selected >= self.scroll_offset.saturating_add(viewport) {
                self.scroll_offset = self.selected + 1 - viewport;
            }
            self.scroll_offset = self.scroll_offset.min(len.saturating_sub(viewport));
        }

        fn visible_range(&self, len: usize, viewport: usize) -> std::ops::Range<usize> {
            let start = self.scroll_offset.min(len);
            start..(start + viewport).min(len)
        }

        fn render_projects(&self, viewport: usize) {
            println!("Projects");
            let projects = self.projects();
            if projects.is_empty() {
                println!(
                    "  {}",
                    if self.status.is_empty() {
                        "No sessions found"
                    } else {
                        &self.status
                    }
                );
            }
            for (index, summary) in projects
                .iter()
                .enumerate()
                .skip(self.visible_range(projects.len(), viewport).start)
                .take(viewport)
            {
                let marker = if index == self.selected { ">" } else { " " };
                let running = if summary.running_count > 0 {
                    format!(" ●{}", summary.running_count)
                } else {
                    String::new()
                };
                println!(
                    "{marker} {:<28} {:>3}{}",
                    summary.project.name, summary.session_count, running
                );
            }
        }

        fn render_sessions(&self, viewport: usize) {
            let name = self.project_id.as_deref().unwrap_or("Project");
            println!("{name}");
            let sessions = self.sessions_for_current_project();
            for (index, session) in sessions
                .iter()
                .enumerate()
                .skip(self.visible_range(sessions.len(), viewport).start)
                .take(viewport)
            {
                println!(
                    "{} {} {:<10} {}",
                    if index == self.selected { ">" } else { " " },
                    status_marker(session),
                    session.agent,
                    session.title
                );
            }
        }

        fn render_search(&self, viewport: usize) {
            println!("Search: {}", self.search_query);
            let sessions = self.search_results();
            if sessions.is_empty() {
                println!("  No matching sessions");
            }
            for (index, session) in sessions
                .iter()
                .enumerate()
                .skip(self.visible_range(sessions.len(), viewport).start)
                .take(viewport)
            {
                println!(
                    "{} {} {:<10} {} [{}]",
                    if index == self.selected { ">" } else { " " },
                    status_marker(session),
                    session.agent,
                    session.title,
                    session.directory.display()
                );
            }
        }

        fn render_new_session(&self, viewport: usize) {
            let project = self.project_id.as_deref().unwrap_or("Project");
            println!("New Agent Session in {project}");
            for (index, agent) in ["Codex", "OpenCode"]
                .iter()
                .enumerate()
                .skip(self.visible_range(2, viewport).start)
                .take(viewport)
            {
                println!(
                    "{} {}",
                    if index == self.selected { ">" } else { " " },
                    agent
                );
            }
        }
    }

    fn status_marker(session: &AiSession) -> &'static str {
        match session.status {
            SessionStatus::Running => "●",
            SessionStatus::Historical => "○",
        }
    }

    fn command_error(stderr: Vec<u8>) -> String {
        let message = String::from_utf8_lossy(&stderr).trim().to_string();
        if message.is_empty() {
            "Indexer command failed".into()
        } else {
            message
        }
    }
}

#[cfg(feature = "wasm")]
use plugin::AiSessionPlugin;
#[cfg(feature = "wasm")]
use zellij_tile::prelude::*;

#[cfg(feature = "wasm")]
register_plugin!(AiSessionPlugin);

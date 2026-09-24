#[cfg(not(feature = "wasm"))]
fn main() {}

#[cfg(feature = "wasm")]
mod plugin {
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    use std::path::PathBuf;

    use chrono::{Local, TimeZone, Utc};
    use unicode_width::UnicodeWidthChar;
    use zellij_ai_session_core::{
        AiSession, CommandSpec, IndexSnapshot, ProjectSort, ProjectSummary, RuntimeConfidence,
        RuntimeRef, SessionPreview, SessionSort, SessionStatus, search_key, sort_projects,
        sort_sessions,
    };
    use zellij_tile::prelude::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    enum View {
        #[default]
        Tree,
        Search,
        ProjectForm,
        ParentPicker,
        RenameSession,
    }

    #[derive(Clone)]
    enum TreeItem {
        Project(ProjectSummary),
        Session(AiSession, usize),
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
        collapsed: BTreeSet<String>,
        child_id: Option<String>,
        rename_id: Option<String>,
        rename_title: String,
        form_name: String,
        form_root: String,
        form_field: usize,
        search_query: String,
        status: String,
        status_after_refresh: Option<String>,
        indexer: String,
        open_mode: OpenMode,
        scroll_offset: usize,
        visible_rows: usize,
        preview: Option<SessionPreview>,
        preview_error: Option<String>,
        preview_error_id: Option<String>,
        preview_pending: Option<String>,
        preview_started_at_ms: Option<i64>,
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
            self.refresh();
        }

        fn update(&mut self, event: Event) -> bool {
            let changed = match event {
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
                    self.check_preview_timeout();
                    true
                }
                _ => false,
            };
            if changed && self.view == View::Tree {
                self.request_selected_preview();
            }
            changed
        }

        fn render(&mut self, rows: usize, cols: usize) {
            print!("\x1b[2J\x1b[H");
            println!("AI Sessions{}", " ".repeat(cols.saturating_sub(12)));
            println!();

            let viewport = rows.saturating_sub(7).max(1);
            let now_ms = Utc::now().timestamp_millis();
            self.visible_rows = if matches!(self.view, View::Tree) && cols < 72 {
                (viewport / 2).max(2)
            } else {
                viewport
            };
            self.ensure_visible(self.visible_rows);
            match self.view {
                View::Tree => {
                    self.render_tree(viewport, now_ms, cols);
                }
                View::Search => self.render_search(viewport, now_ms),
                View::ProjectForm => self.render_project_form(),
                View::ParentPicker => self.render_parent_picker(viewport),
                View::RenameSession => self.render_rename_session(),
            }

            println!();
            println!("{}", "─".repeat(cols.max(1)));
            match self.view {
                View::Tree => println!(
                    "Enter open   Space toggle   ←/→ collapse/expand   C/E all   g/G/M first/mid/last   p project   m parent   R rename   / search   r refresh   q close"
                ),
                View::Search => println!("Type or paste to search   Backspace erase   Esc back"),
                View::ProjectForm => println!("Tab next field   Enter save   Esc cancel"),
                View::ParentPicker => println!("Enter set parent   Esc cancel"),
                View::RenameSession => println!("Enter rename   Esc cancel"),
            }
            if !self.status.is_empty() {
                println!("{}", self.status);
            }
        }
    }

    impl AiSessionPlugin {
        fn refresh(&mut self) {
            self.preview = None;
            self.preview_error = None;
            self.preview_error_id = None;
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
                    let agent = zellij_ai_session_core::AGENT_META
                        .iter()
                        .find(|meta| command.contains(meta.command))
                        .map(|meta| meta.command);
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
                if !session.native_available {
                    session.runtime = None;
                    session.status = SessionStatus::Historical;
                    continue;
                }
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
            let action = context.get("action").map(String::as_str);
            if action == Some("preview") {
                let id = context.get("session_id").cloned().unwrap_or_default();
                eprintln!(
                    "[session-preview] result key={id} exit={exit_code:?} stdout_bytes={} stderr_bytes={}",
                    stdout.len(),
                    stderr.len()
                );
                if self.preview_pending.as_deref() == Some(&id) {
                    self.preview_pending = None;
                    self.preview_started_at_ms = None;
                }
                if self.selected_session_id().as_deref() == Some(&id) {
                    if exit_code == Some(0) {
                        match serde_json::from_slice::<SessionPreview>(&stdout) {
                            Ok(preview) if self.tree_items().get(self.selected).is_some_and(|item| {
                                matches!(item, TreeItem::Session(session, _) if session.id == id && session.agent_session_id == preview.session_id)
                            }) => {
                                self.preview = Some(preview);
                                self.preview_error = None;
                                self.preview_error_id = None;
                            }
                            Err(error) => {
                                self.preview_error = Some(format!("Invalid preview response: {error}"));
                                self.preview_error_id = Some(id);
                            }
                            Ok(_) => {
                                self.preview_error = Some("Preview response belongs to another native session".into());
                                self.preview_error_id = Some(id);
                            }
                        }
                    } else {
                        self.preview_error = Some(format!(
                            "Indexer exit {:?}: {}",
                            exit_code,
                            command_error(stderr)
                        ));
                        self.preview_error_id = Some(id);
                    }
                }
                return true;
            }
            if matches!(action, Some("resume" | "new")) {
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
            if matches!(action, Some("create-project" | "set-parent" | "rename-session")) {
                if exit_code == Some(0) {
                    self.status = if action == Some("create-project") {
                        "Project saved".into()
                    } else if action == Some("rename-session") {
                        "Native session renamed".into()
                    } else {
                        "Parent updated".into()
                    };
                    self.view = View::Tree;
                    if action == Some("rename-session") {
                        self.status_after_refresh = Some(self.status.clone());
                    }
                    self.refresh();
                } else {
                    self.status = command_error(stderr);
                }
                return true;
            }

            if exit_code == Some(0) {
                match serde_json::from_slice::<IndexSnapshot>(&stdout) {
                    Ok(snapshot) => {
                        self.snapshot = Some(snapshot);
                        self.status = self.status_after_refresh.take().unwrap_or_default();
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
            if matches!(self.view, View::ProjectForm) {
                return self.handle_project_form_key(key.bare_key);
            }
            if matches!(self.view, View::RenameSession) {
                return self.handle_rename_key(key.bare_key);
            }
            if !key.has_no_modifiers() {
                return false;
            }
            match self.view {
                View::Search | View::ProjectForm | View::RenameSession => false,
                View::Tree => match key.bare_key {
                    BareKey::Down | BareKey::Char('j') => {
                        self.move_selection(1);
                        true
                    }
                    BareKey::Up | BareKey::Char('k') => {
                        self.move_selection(-1);
                        true
                    }
                    BareKey::Char('C') => {
                        self.collapse_all_projects();
                        true
                    }
                    BareKey::Char('E') => {
                        self.expand_all_projects();
                        true
                    }
                    BareKey::Char('g') => {
                        self.jump_to_visible_item(0);
                        true
                    }
                    BareKey::Char('G') => {
                        self.jump_to_visible_item(self.tree_items().len().saturating_sub(1));
                        true
                    }
                    BareKey::Char('M') => {
                        let len = self.tree_items().len();
                        self.jump_to_visible_item(len.saturating_sub(1) / 2);
                        true
                    }
                    BareKey::Enter => {
                        self.activate_tree_item();
                        true
                    }
                    BareKey::Char(' ') => {
                        self.toggle_selected_expandable();
                        true
                    }
                    BareKey::Left => {
                        self.collapse_selected();
                        true
                    }
                    BareKey::Right => {
                        self.expand_selected();
                        true
                    }
                    BareKey::Char('p') => {
                        self.start_project_form();
                        true
                    }
                    BareKey::Char('m') => {
                        self.start_parent_picker();
                        true
                    }
                    BareKey::Char('R') => {
                        self.start_rename_session();
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
                    BareKey::Char('q') | BareKey::Esc => {
                        close_focus();
                        false
                    }
                    _ => false,
                },
                View::ParentPicker => match key.bare_key {
                    BareKey::Down | BareKey::Char('j') => {
                        self.move_selection(1);
                        true
                    }
                    BareKey::Up | BareKey::Char('k') => {
                        self.move_selection(-1);
                        true
                    }
                    BareKey::Enter => {
                        self.commit_parent();
                        true
                    }
                    BareKey::Esc => {
                        self.view = View::Tree;
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
                    self.view = View::Tree;
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
            if matches!(self.view, View::RenameSession) {
                self.rename_title.extend(text.chars().filter(|c| !c.is_control()));
                return true;
            }
            if matches!(self.view, View::ProjectForm) {
                self.form_value_mut()
                    .push_str(text.trim_end_matches(['\n', '\r']));
                return true;
            }
            if !matches!(self.view, View::Search) {
                return false;
            }
            self.search_query.extend(
                text.chars()
                    .filter(|character| !matches!(character, '\n' | '\r')),
            );
            self.selected = 0;
            self.scroll_offset = 0;
            true
        }

        fn form_value_mut(&mut self) -> &mut String {
            if self.form_field == 0 {
                &mut self.form_name
            } else {
                &mut self.form_root
            }
        }

        fn handle_project_form_key(&mut self, key: BareKey) -> bool {
            match key {
                BareKey::Esc => self.view = View::Tree,
                BareKey::Tab => self.form_field = (self.form_field + 1) % 2,
                BareKey::Backspace => {
                    self.form_value_mut().pop();
                }
                BareKey::Enter => {
                    if self.form_name.trim().is_empty() || self.form_root.trim().is_empty() {
                        self.status = "Project name and directory are required".into();
                    } else {
                        let mut context = BTreeMap::new();
                        context.insert("action".into(), "create-project".into());
                        run_command(
                            &[
                                self.indexer.as_str(),
                                "create-project",
                                "--name",
                                self.form_name.trim(),
                                "--root",
                                self.form_root.trim(),
                            ],
                            context,
                        );
                        self.status = "Saving project…".into();
                    }
                }
                BareKey::Char(c) => self.form_value_mut().push(c),
                _ => return false,
            }
            true
        }

        fn start_project_form(&mut self) {
            let root = match self.tree_items().get(self.selected) {
                Some(TreeItem::Project(summary)) => Some(summary.project.root_directory.clone()),
                Some(TreeItem::Session(session, _)) => Some(session.directory.clone()),
                None => None,
            }
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            self.form_name = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Project".into());
            self.form_root = root.to_string_lossy().into_owned();
            self.form_field = 0;
            self.view = View::ProjectForm;
        }

        fn start_rename_session(&mut self) {
            let Some(TreeItem::Session(session, _)) = self.tree_items().get(self.selected).cloned() else {
                self.status = "Select a session to rename".into();
                return;
            };
            if !session.native_available {
                self.status = "Native session is unavailable".into();
                return;
            }
            if !matches!(session.agent, zellij_ai_session_core::AgentKind::Codex | zellij_ai_session_core::AgentKind::OpenCode) {
                self.status = format!("{} does not support native renaming", session.agent);
                return;
            }
            self.rename_id = Some(session.id);
            self.rename_title = session.title;
            self.view = View::RenameSession;
        }

        fn handle_rename_key(&mut self, key: BareKey) -> bool {
            match key {
                BareKey::Esc => self.view = View::Tree,
                BareKey::Backspace => { self.rename_title.pop(); },
                BareKey::Enter => {
                    let title = self.rename_title.trim();
                    if title.is_empty() {
                        self.status = "Session name cannot be empty".into();
                    } else if let Some(id) = &self.rename_id {
                        let mut context = BTreeMap::new();
                        context.insert("action".into(), "rename-session".into());
                        run_command(&[self.indexer.as_str(), "rename-session", "--id", id, "--title", title], context);
                        self.status = "Renaming native session…".into();
                    }
                }
                BareKey::Char(c) if !c.is_control() => self.rename_title.push(c),
                _ => return false,
            }
            true
        }

        fn activate_tree_item(&mut self) {
            match self.tree_items().get(self.selected).cloned() {
                Some(TreeItem::Project(summary)) => self.toggle(&summary.project.id),
                Some(TreeItem::Session(session, _)) => self.resume_or_open(session),
                None => {}
            }
        }

        fn selected_expandable_id(&self) -> Option<String> {
            match self.tree_items().get(self.selected) {
                Some(TreeItem::Project(summary)) => Some(summary.project.id.clone()),
                Some(TreeItem::Session(session, _))
                    if self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot
                            .sessions
                            .iter()
                            .any(|child| child.parent_id.as_deref() == Some(&session.id))
                    }) =>
                {
                    Some(session.id.clone())
                }
                _ => None,
            }
        }

        fn toggle(&mut self, id: &str) {
            if !self.collapsed.insert(id.to_owned()) {
                self.collapsed.remove(id);
            }
            self.clamp_selection();
        }

        fn collapse_selected(&mut self) {
            if let Some(id) = self.selected_expandable_id() {
                self.collapsed.insert(id);
            }
            self.clamp_selection();
        }

        fn expand_selected(&mut self) {
            if let Some(id) = self.selected_expandable_id() {
                self.collapsed.remove(&id);
            }
        }

        fn toggle_selected_expandable(&mut self) {
            if let Some(id) = self.selected_expandable_id() {
                self.toggle(&id);
            }
        }

        fn collapse_all_projects(&mut self) {
            let selected_project_id = match self.tree_items().get(self.selected) {
                Some(TreeItem::Project(summary)) => Some(summary.project.id.clone()),
                Some(TreeItem::Session(session, _)) => Some(session.project_id.clone()),
                None => None,
            };
            let project_ids: Vec<String> = self
                .projects()
                .into_iter()
                .map(|summary| summary.project.id.clone())
                .collect();
            self.collapsed.extend(project_ids.iter().cloned());

            let visible = self.tree_items();
            self.selected = selected_project_id
                .and_then(|project_id| {
                    visible.iter().position(|item| {
                        matches!(item, TreeItem::Project(summary) if summary.project.id == project_id)
                    })
                })
                .unwrap_or(0);
            self.ensure_visible(self.last_viewport());
        }

        fn expand_all_projects(&mut self) {
            let project_ids: Vec<String> = self
                .projects()
                .into_iter()
                .map(|summary| summary.project.id.clone())
                .collect();
            for project_id in project_ids {
                self.collapsed.remove(&project_id);
            }
            self.clamp_selection();
            self.ensure_visible(self.last_viewport());
        }

        fn jump_to_visible_item(&mut self, index: usize) {
            let len = self.tree_items().len();
            if len == 0 {
                self.selected = 0;
                self.scroll_offset = 0;
                return;
            }
            self.selected = index.min(len - 1);
            self.ensure_visible(self.last_viewport());
        }

        fn last_viewport(&self) -> usize {
            self.visible_rows.max(1)
        }

        fn start_parent_picker(&mut self) {
            let Some(TreeItem::Session(child, _)) = self.tree_items().get(self.selected).cloned()
            else {
                self.status = "Select a session first".into();
                return;
            };
            self.child_id = Some(child.id);
            self.view = View::ParentPicker;
            self.selected = 0;
            self.scroll_offset = 0;
        }

        fn parent_candidates(&self) -> Vec<Option<AiSession>> {
            let Some(snapshot) = &self.snapshot else {
                return Vec::new();
            };
            let Some(child) = snapshot
                .sessions
                .iter()
                .find(|s| Some(&s.id) == self.child_id.as_ref())
            else {
                return Vec::new();
            };
            let mut candidates: Vec<_> = snapshot
                .sessions
                .iter()
                .filter(|s| s.project_id == child.project_id && s.id != child.id)
                .cloned()
                .collect();
            sort_sessions(&mut candidates, SessionSort::UpdatedDesc);
            std::iter::once(None)
                .chain(candidates.into_iter().map(Some))
                .collect()
        }

        fn commit_parent(&mut self) {
            let Some(child_id) = self.child_id.as_deref() else {
                return;
            };
            let candidates = self.parent_candidates();
            let Some(choice) = candidates.get(self.selected) else {
                return;
            };
            let mut args = vec![self.indexer.as_str(), "set-parent", "--id", child_id];
            if let Some(parent) = choice {
                args.extend(["--parent-id", parent.id.as_str()]);
            }
            let mut context = BTreeMap::new();
            context.insert("action".into(), "set-parent".into());
            run_command(&args, context);
            self.status = "Saving parent…".into();
        }

        fn open_selected_search_session(&mut self) {
            let Some(session) = self.search_results().get(self.selected).cloned() else {
                return;
            };
            self.resume_or_open(session);
        }

        fn resume_or_open(&mut self, session: AiSession) {
            if !session.native_available {
                self.status = format!("Native session {} is unavailable", session.id);
                return;
            }
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
            let mut projects = self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.projects.clone())
                .unwrap_or_default();
            sort_projects(&mut projects, ProjectSort::LatestSessionUpdatedDesc);
            projects
        }

        fn tree_items(&self) -> Vec<TreeItem> {
            let Some(snapshot) = &self.snapshot else {
                return Vec::new();
            };
            let mut items = Vec::new();
            for summary in self.projects() {
                let project_id = summary.project.id.clone();
                items.push(TreeItem::Project(summary));
                if self.collapsed.contains(&project_id) {
                    continue;
                }
                let mut sessions: Vec<_> = snapshot
                    .sessions
                    .iter()
                    .filter(|s| s.project_id == project_id)
                    .cloned()
                    .collect();
                sort_sessions(&mut sessions, SessionSort::UpdatedDesc);
                let ids: HashSet<_> = sessions.iter().map(|s| s.id.as_str()).collect();
                let roots: Vec<_> = sessions
                    .iter()
                    .filter(|s| s.parent_id.as_deref().is_none_or(|p| !ids.contains(p)))
                    .cloned()
                    .collect();
                let mut visited = HashSet::new();
                for root in roots {
                    self.append_session_tree(&sessions, &root, 1, &mut visited, &mut items);
                }
            }
            items
        }

        fn append_session_tree(
            &self,
            sessions: &[AiSession],
            session: &AiSession,
            depth: usize,
            visited: &mut HashSet<String>,
            items: &mut Vec<TreeItem>,
        ) {
            if !visited.insert(session.id.clone()) {
                return;
            }
            items.push(TreeItem::Session(session.clone(), depth));
            if self.collapsed.contains(&session.id) {
                return;
            }
            for child in sessions
                .iter()
                .filter(|s| s.parent_id.as_deref() == Some(&session.id))
            {
                self.append_session_tree(sessions, child, depth + 1, visited, items);
            }
        }

        fn search_results(&self) -> Vec<AiSession> {
            let Some(snapshot) = &self.snapshot else {
                return Vec::new();
            };
            let query = search_key(self.search_query.trim());
            let mut sessions: Vec<AiSession> = snapshot
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
                .collect();
            sort_sessions(&mut sessions, SessionSort::UpdatedDesc);
            sessions
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
                View::Tree => self.tree_items().len(),
                View::Search => self.search_results().len(),
                View::ProjectForm => 0,
                View::ParentPicker => self.parent_candidates().len(),
                View::RenameSession => 0,
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

        fn selected_session_id(&self) -> Option<String> {
            match self.tree_items().get(self.selected) {
                Some(TreeItem::Session(session, _)) => Some(session.id.clone()),
                _ => None,
            }
        }

        fn request_selected_preview(&mut self) {
            let Some(TreeItem::Session(session, _)) = self.tree_items().get(self.selected).cloned()
            else {
                return;
            };
            if !session.native_available
                || self
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.session_id == session.agent_session_id)
                || self.preview_error_id.as_deref() == Some(&session.id)
                || self.preview_pending.is_some()
            {
                return;
            }
            let mut context = BTreeMap::new();
            context.insert("action".into(), "preview".into());
            context.insert("session_id".into(), session.id.clone());
            eprintln!(
                "[session-preview] request key={} native={}",
                session.id, session.agent_session_id
            );
            run_command(
                &[
                    self.indexer.as_str(),
                    "preview",
                    "--agent",
                    session.agent.command_name(),
                    "--session-id",
                    session.agent_session_id.as_str(),
                ],
                context,
            );
            self.preview_pending = Some(session.id);
            self.preview_started_at_ms = Some(Utc::now().timestamp_millis());
            set_timeout(1.0);
        }

        fn check_preview_timeout(&mut self) {
            let Some(id) = self.preview_pending.clone() else {
                return;
            };
            let elapsed = Utc::now()
                .timestamp_millis()
                .saturating_sub(self.preview_started_at_ms.unwrap_or_default());
            if elapsed >= 10_000 {
                eprintln!("[session-preview] timeout key={id} elapsed_ms={elapsed}");
                self.preview_pending = None;
                self.preview_started_at_ms = None;
                self.preview_error_id = Some(id);
                self.preview_error = Some(
                    "No preview command result after 10 seconds; check the indexer path, permissions, or Zellij logs".into(),
                );
            } else {
                set_timeout(1.0);
            }
        }

        fn render_tree(&self, viewport: usize, now_ms: i64, cols: usize) {
            let items = self.tree_items();
            let split = cols >= 72;
            let tree_viewport = if split {
                viewport
            } else {
                (viewport / 2).max(2)
            };
            // Keep the session tree as the primary work area while leaving
            // enough room in the details inspector for readable previews.
            let left_width = if split {
                cols.saturating_sub((cols / 4).clamp(40, 60) + 3)
            } else {
                cols
            };
            let right_width = cols.saturating_sub(left_width + 3);
            let mut left = vec!["Tasks".to_string()];
            if items.is_empty() {
                left.push("  No projects found. Press p to create one.".into());
            }
            for (index, item) in items
                .iter()
                .enumerate()
                .skip(self.visible_range(items.len(), tree_viewport).start)
                .take(tree_viewport)
            {
                let marker = if index == self.selected { ">" } else { " " };
                match item {
                    TreeItem::Project(summary) => {
                        let arrow = if self.collapsed.contains(&summary.project.id) {
                            "▸"
                        } else {
                            "▾"
                        };
                        left.push(format!(
                            "{marker} {arrow} {} ({})  {}",
                            summary.project.name,
                            summary.session_count,
                            format_updated_at(summary.latest_updated_at_ms, now_ms)
                        ));
                    }
                    TreeItem::Session(session, depth) => {
                        let has_children = self.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot
                                .sessions
                                .iter()
                                .any(|s| s.parent_id.as_deref() == Some(&session.id))
                        });
                        let arrow = if !has_children {
                            " "
                        } else if self.collapsed.contains(&session.id) {
                            "▸"
                        } else {
                            "▾"
                        };
                        left.push(format!(
                            "{marker} {}{arrow} {} {}  {}{}",
                            "  ".repeat(*depth),
                            status_marker(session),
                            session.agent,
                            session.title,
                            if session.native_available {
                                ""
                            } else {
                                " [native unavailable]"
                            }
                        ));
                    }
                }
            }
            let detail = self.preview_lines(
                items.get(self.selected),
                if split { right_width } else { cols },
                viewport,
            );
            if split {
                for row in 0..viewport + 1 {
                    let line = left.get(row).map(String::as_str).unwrap_or("");
                    let displayed = clip(line, left_width);
                    let pad = left_width
                        .saturating_sub(displayed.chars().map(|c| c.width().unwrap_or(0)).sum());
                    println!(
                        "{}{} │  {} ",
                        displayed,
                        " ".repeat(pad),
                        clip(
                            detail.get(row).map(String::as_str).unwrap_or(""),
                            right_width.saturating_sub(2)
                        )
                    );
                }
            } else {
                for line in left.into_iter().take(tree_viewport + 1) {
                    println!("{}", clip(&line, cols));
                }
                println!("{}", "─".repeat(cols));
                for line in detail
                    .into_iter()
                    .take(viewport.saturating_sub(tree_viewport))
                {
                    println!("{}", clip(&line, cols));
                }
            }
        }

        fn preview_lines(
            &self,
            item: Option<&TreeItem>,
            width: usize,
            height: usize,
        ) -> Vec<String> {
            let mut lines = vec!["Task details".to_string()];
            match item {
                Some(TreeItem::Project(summary)) => {
                    lines.push(summary.project.name.clone());
                    lines.push(format!(
                        "{} sessions · {} running",
                        summary.session_count, summary.running_count
                    ));
                    lines.push(summary.project.root_directory.display().to_string());
                }
                Some(TreeItem::Session(session, _)) => {
                    lines.push(session.title.clone());
                    lines.push(format!(
                        "{} · {}",
                        session.agent,
                        if !session.native_available {
                            "native unavailable"
                        } else if session.status == SessionStatus::Running {
                            "running"
                        } else {
                            "historical"
                        }
                    ));
                    lines.push(session.directory.display().to_string());
                    lines.push(String::new());
                    if !session.native_available {
                        lines.push("Native history is unavailable".into());
                    } else if let Some(preview) = self
                        .preview
                        .as_ref()
                        .filter(|p| p.session_id == session.agent_session_id)
                    {
                        if let Some(note) = &preview.note {
                            lines.push(note.clone());
                        }
                        if !preview.messages.is_empty() {
                            lines.push("Recent messages (newest first)".into());
                        }
                        for message in preview.messages.iter().rev() {
                            lines.push(format!(
                                "{}:",
                                if message.role == "user" {
                                    "User"
                                } else {
                                    "Agent"
                                }
                            ));
                            for line in message.text.lines().take(4) {
                                lines.extend(wrap_text(line, width, 3));
                            }
                            lines.push(String::new());
                        }
                    } else if self.preview_error_id.as_deref() == Some(&session.id) {
                        lines.push(format!(
                            "Preview unavailable: {}",
                            self.preview_error.as_deref().unwrap_or("unknown error")
                        ));
                    } else {
                        let detail = if self.preview_pending.as_deref() == Some(&session.id) {
                            format!("Waiting for indexer result: {}", session.agent_session_id)
                        } else if self.preview_pending.is_some() {
                            "Waiting for previous preview request".into()
                        } else {
                            "Preview request not started".into()
                        };
                        lines.push(format!("Loading native history… {detail}"));
                    }
                }
                None => {}
            }
            lines.truncate(height + 1);
            lines
        }

        fn render_search(&self, viewport: usize, now_ms: i64) {
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
                    "{} {} {:<10} {}{}  {} [{}]",
                    if index == self.selected { ">" } else { " " },
                    status_marker(session),
                    session.agent,
                    session.title,
                    if session.native_available {
                        ""
                    } else {
                        " [native unavailable]"
                    },
                    format_updated_at(session.updated_at_ms, now_ms),
                    session.directory.display()
                );
            }
        }

        fn render_project_form(&self) {
            println!("New project");
            println!(
                "{} Name: {}",
                if self.form_field == 0 { ">" } else { " " },
                self.form_name
            );
            println!(
                "{} Directory: {}",
                if self.form_field == 1 { ">" } else { " " },
                self.form_root
            );
        }

        fn render_rename_session(&self) {
            println!("Rename native session {}", self.rename_id.as_deref().unwrap_or(""));
            println!("> Name: {}", self.rename_title);
        }

        fn render_parent_picker(&self, viewport: usize) {
            println!("Set parent for {}", self.child_id.as_deref().unwrap_or(""));
            let candidates = self.parent_candidates();
            for (index, candidate) in candidates
                .iter()
                .enumerate()
                .skip(self.visible_range(candidates.len(), viewport).start)
                .take(viewport)
            {
                let label = candidate
                    .as_ref()
                    .map(|s| format!("{} · {} ({})", s.agent, s.title, s.id))
                    .unwrap_or_else(|| "No parent (root session)".into());
                println!(
                    "{} {}",
                    if index == self.selected { ">" } else { " " },
                    label
                );
            }
        }
    }

    fn status_marker(session: &AiSession) -> &'static str {
        if !session.native_available {
            return "×";
        }
        match session.status {
            SessionStatus::Running => "●",
            SessionStatus::Historical => "○",
        }
    }

    fn format_updated_at(updated_at_ms: Option<i64>, now_ms: i64) -> String {
        let Some(updated_at_ms) = updated_at_ms else {
            return "时间未知".into();
        };

        let elapsed_ms = now_ms.saturating_sub(updated_at_ms);
        if elapsed_ms >= 0 {
            let minutes = elapsed_ms / 60_000;
            if minutes == 0 {
                return "刚刚".into();
            }
            if minutes < 60 {
                return format!("{minutes}分钟前");
            }

            let hours = minutes / 60;
            if hours < 24 {
                return format!("{hours}小时前");
            }

            let days = hours / 24;
            if days < 7 {
                return format!("{days}天前");
            }
        }

        Local
            .timestamp_millis_opt(updated_at_ms)
            .single()
            .map(|datetime| datetime.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "时间未知".into())
    }

    fn command_error(stderr: Vec<u8>) -> String {
        let message = String::from_utf8_lossy(&stderr).trim().to_string();
        if message.is_empty() {
            "Indexer command failed".into()
        } else {
            message
        }
    }

    fn clip(text: &str, width: usize) -> String {
        let mut out = String::new();
        let mut used = 0;
        for c in text.chars() {
            if c.is_control() {
                continue;
            }
            let char_width = c.width().unwrap_or(0);
            if used + char_width > width {
                break;
            }
            out.push(c);
            used += char_width;
        }
        out
    }

    fn wrap_text(text: &str, width: usize, max_lines: usize) -> Vec<String> {
        let width = width.max(1);
        let mut lines = Vec::new();
        let mut chars = text.chars().peekable();
        for _ in 0..max_lines {
            let mut part = String::new();
            let mut used = 0;
            while let Some(&c) = chars.peek() {
                if c.is_control() {
                    chars.next();
                    continue;
                }
                let char_width = c.width().unwrap_or(0);
                if used + char_width > width {
                    break;
                }
                part.push(c);
                chars.next();
                used += char_width;
            }
            if part.is_empty() {
                break;
            }
            lines.push(part);
        }
        if chars.peek().is_some() {
            if let Some(last) = lines.last_mut() {
                last.push('…');
            }
        }
        lines
    }
}

#[cfg(feature = "wasm")]
use plugin::AiSessionPlugin;
#[cfg(feature = "wasm")]
use zellij_tile::prelude::*;

#[cfg(feature = "wasm")]
register_plugin!(AiSessionPlugin);

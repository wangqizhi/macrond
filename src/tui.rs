use crate::config;
use crate::daemon;
use crate::model::{CommandConfig, JobConfig, Repeat, ScheduleConfig};
use crate::paths::AppPaths;
use crate::scheduler;
use anyhow::{Context, Result, bail};
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::Command as StdCommand;
use std::time::{Duration, Instant};

pub fn run_tui(paths: &AppPaths) -> Result<()> {
    let mut ui = UiState::load(paths)?;
    let mut terminal = ratatui::init();
    let mut last_auto_refresh = Instant::now();

    let mut quit = false;
    while !quit {
        if last_auto_refresh.elapsed() >= Duration::from_secs(1) {
            let _ = ui.refresh_runtime(paths);
            last_auto_refresh = Instant::now();
        }
        terminal.draw(|f| render(f, &ui))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            quit = ui.on_key(paths, key)?;
        }
    }

    ratatui::restore();
    Ok(())
}

struct UiState {
    jobs: Vec<JobConfig>,
    history_runs: Vec<String>,
    daemon_pid: Option<i32>,
    selected: usize,
    history_selected: usize,
    focus: ListFocus,
    message: String,
    mode: UiMode,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum ListFocus {
    Jobs,
    History,
}

enum UiMode {
    List,
    Edit(EditState),
    ConfirmDelete { job_id: String },
    ConfirmDiscard { edit: Box<EditState> },
}

struct EditState {
    form: JobForm,
    selected: usize,
    dirty: bool,
    input: Option<InputState>,
    message: String,
}

#[derive(Clone)]
struct InputState {
    field: EditField,
    kind: InputKind,
}

#[derive(Clone)]
enum InputKind {
    Text {
        value: String,
        cursor: usize,
        suggest: Option<SuggestState>,
    },
    Select { options: Vec<String>, selected: usize },
}

#[derive(Clone)]
struct SuggestState {
    options: Vec<String>,
    selected: usize,
    kind: SuggestKind,
}

#[derive(Clone)]
enum SuggestKind {
    WorkingDir { base: String },
    ProgramPath { replace_start: usize, replace_end: usize },
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum ScheduleKind {
    Cron,
    Simple,
}

#[derive(Clone)]
struct JobForm {
    id: String,
    name: String,
    enabled: bool,
    schedule_kind: ScheduleKind,
    cron_expression: String,
    repeat: Repeat,
    time: String,
    weekday: u8,
    day: u8,
    once_at: String,
    program: String,
    args: String,
    working_dir: String,
    env_json: String,
    timeout_seconds: String,
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum EditField {
    Name,
    Enabled,
    ScheduleKind,
    CronExpression,
    Repeat,
    Time,
    Weekday,
    Day,
    OnceAt,
    Program,
    Args,
    WorkingDir,
    EnvJson,
    Timeout,
}

impl UiState {
    fn load(paths: &AppPaths) -> Result<Self> {
        let jobs = config::load_jobs(&paths.jobs_dir).unwrap_or_default();
        let history_runs = load_history_runs(&paths.logs_dir).unwrap_or_default();
        let daemon_pid = daemon::daemon_running(paths).ok().flatten();
        Ok(Self {
            jobs,
            history_runs,
            daemon_pid,
            selected: 0,
            history_selected: 0,
            focus: ListFocus::Jobs,
            message: "Ready".to_string(),
            mode: UiMode::List,
        })
    }

    fn reload(&mut self, paths: &AppPaths) -> Result<()> {
        self.jobs = config::load_jobs(&paths.jobs_dir).context("reload jobs failed")?;
        self.history_runs = load_history_runs(&paths.logs_dir).unwrap_or_default();
        self.daemon_pid = daemon::daemon_running(paths).ok().flatten();
        if self.jobs.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.jobs.len() {
            self.selected = self.jobs.len() - 1;
        }
        if self.history_runs.is_empty() {
            self.history_selected = 0;
        } else if self.history_selected >= self.history_runs.len() {
            self.history_selected = self.history_runs.len() - 1;
        }
        Ok(())
    }

    fn refresh_runtime(&mut self, paths: &AppPaths) -> Result<()> {
        self.history_runs = load_history_runs(&paths.logs_dir).unwrap_or_default();
        self.daemon_pid = daemon::daemon_running(paths).ok().flatten();
        self.jobs = config::load_jobs(&paths.jobs_dir).context("refresh jobs failed")?;
        if self.jobs.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.jobs.len() {
            self.selected = self.jobs.len() - 1;
        }
        if self.history_runs.is_empty() {
            self.history_selected = 0;
        } else if self.history_selected >= self.history_runs.len() {
            self.history_selected = self.history_runs.len() - 1;
        }
        Ok(())
    }

    fn selected_job(&self) -> Option<&JobConfig> {
        self.jobs.get(self.selected)
    }

    fn next(&mut self) {
        match self.focus {
            ListFocus::Jobs => {
                if self.jobs.is_empty() {
                    return;
                }
                self.selected = (self.selected + 1) % self.jobs.len();
            }
            ListFocus::History => {
                if self.history_runs.is_empty() {
                    return;
                }
                self.history_selected = (self.history_selected + 1) % self.history_runs.len();
            }
        }
    }

    fn previous(&mut self) {
        match self.focus {
            ListFocus::Jobs => {
                if self.jobs.is_empty() {
                    return;
                }
                if self.selected == 0 {
                    self.selected = self.jobs.len() - 1;
                } else {
                    self.selected -= 1;
                }
            }
            ListFocus::History => {
                if self.history_runs.is_empty() {
                    return;
                }
                if self.history_selected == 0 {
                    self.history_selected = self.history_runs.len() - 1;
                } else {
                    self.history_selected -= 1;
                }
            }
        }
    }

    fn on_key(&mut self, paths: &AppPaths, key: KeyEvent) -> Result<bool> {
        let mode = std::mem::replace(&mut self.mode, UiMode::List);
        match mode {
            UiMode::List => self.on_key_list(paths, key),
            UiMode::ConfirmDelete { job_id } => self.on_key_confirm_delete(paths, key, job_id),
            UiMode::ConfirmDiscard { edit } => self.on_key_confirm_discard(key, *edit),
            UiMode::Edit(edit) => self.on_key_edit(paths, key, edit),
        }
    }

    fn on_key_list(&mut self, paths: &AppPaths, key: KeyEvent) -> Result<bool> {
        self.daemon_pid = daemon::daemon_running(paths).ok().flatten();
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => self.next(),
            KeyCode::Char('k') | KeyCode::Up => self.previous(),
            KeyCode::Left | KeyCode::Char('h') => {
                self.focus = ListFocus::Jobs;
                self.message = "Focus: Jobs".to_string();
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.focus = ListFocus::History;
                self.message = "Focus: History Runs".to_string();
            }
            KeyCode::Char('r') => {
                self.reload(paths)?;
                self.message = format!("Reloaded {} jobs", self.jobs.len());
            }
            KeyCode::Char('a') => {
                if self.focus != ListFocus::Jobs {
                    self.message = "Switch focus to Jobs to add/edit/delete".to_string();
                    return Ok(false);
                }
                let mut id = generate_job_id();
                while job_file_path(&paths.jobs_dir, &id).exists() {
                    id = generate_job_id();
                }
                self.mode = UiMode::Edit(EditState::new(JobForm::new(id), "Creating new job"));
            }
            KeyCode::Char('s') => {
                if self.focus != ListFocus::Jobs {
                    self.message = "Switch focus to Jobs to toggle job".to_string();
                    return Ok(false);
                }
                if let Some(job_id) = self.selected_job().map(|j| j.id.clone()) {
                    let current = load_job_by_id(&paths.jobs_dir, &job_id)?;
                    let next_enabled = !current.enabled;
                    set_job_enabled(paths, &job_id, next_enabled)?;
                    self.reload(paths)?;
                    if next_enabled {
                        if self.daemon_pid.is_some() {
                            self.message = format!("Started job {job_id}");
                        } else {
                            self.message = format!("Started job {job_id}, but daemon is stopped");
                        }
                    } else {
                        self.message = format!("Stopped job {job_id}");
                    }
                } else {
                    self.message = "No job selected".to_string();
                }
            }
            KeyCode::Char('t') => {
                if self.focus != ListFocus::Jobs {
                    self.message = "Switch focus to Jobs to test job".to_string();
                    return Ok(false);
                }
                if let Some(job_id) = self.selected_job().map(|j| j.id.clone()) {
                    self.message = run_test(paths, &job_id)?;
                } else {
                    self.message = "No job selected".to_string();
                }
            }
            KeyCode::Char('S') => {
                self.message = daemon_command(paths, "start")?;
                self.reload(paths)?;
            }
            KeyCode::Char('X') => {
                self.message = daemon_command(paths, "stop")?;
                self.reload(paths)?;
            }
            KeyCode::Char('e') => {
                if self.focus != ListFocus::Jobs {
                    self.message = "Switch focus to Jobs to edit job".to_string();
                    return Ok(false);
                }
                if let Some(job) = self.selected_job() {
                    self.mode = UiMode::Edit(EditState::new(JobForm::from_job(job), "Editing job"));
                } else {
                    self.message = "No job selected".to_string();
                }
            }
            KeyCode::Enter => {
                if self.focus == ListFocus::Jobs {
                    if let Some(job) = self.selected_job() {
                        self.mode = UiMode::Edit(EditState::new(JobForm::from_job(job), "Editing job"));
                    } else {
                        self.message = "No job selected".to_string();
                    }
                } else {
                    self.message = self
                        .history_runs
                        .get(self.history_selected)
                        .cloned()
                        .unwrap_or_else(|| "No history line selected".to_string());
                }
            }
            KeyCode::Char('d') => {
                if self.focus != ListFocus::Jobs {
                    self.message = "Switch focus to Jobs to delete job".to_string();
                    return Ok(false);
                }
                if let Some(job) = self.selected_job() {
                    self.mode = UiMode::ConfirmDelete {
                        job_id: job.id.clone(),
                    };
                } else {
                    self.message = "No job selected".to_string();
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn on_key_confirm_delete(&mut self, paths: &AppPaths, key: KeyEvent, job_id: String) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') => {
                let path = job_file_path(&paths.jobs_dir, &job_id);
                if path.exists() {
                    fs::remove_file(path)?;
                    self.reload(paths)?;
                    self.message = format!("Deleted job {job_id}");
                } else {
                    self.message = format!("Job file not found for {job_id}");
                }
                self.mode = UiMode::List;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.mode = UiMode::List;
                self.message = "Delete canceled".to_string();
            }
            _ => {}
        }
        Ok(false)
    }

    fn on_key_confirm_discard(&mut self, key: KeyEvent, edit: EditState) -> Result<bool> {
        match key.code {
            KeyCode::Char('y') => {
                self.mode = UiMode::List;
                self.message = "Discarded unsaved changes".to_string();
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.mode = UiMode::Edit(edit);
            }
            _ => {}
        }
        Ok(false)
    }

    fn on_key_edit(&mut self, paths: &AppPaths, key: KeyEvent, mut edit: EditState) -> Result<bool> {
        if let Some(mut input) = edit.input.take() {
            match &mut input.kind {
                InputKind::Text {
                    value,
                    cursor,
                    suggest,
                } => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        value.clear();
                        *cursor = 0;
                        *suggest = suggest_for_input(input.field, value, &edit.form.working_dir);
                        edit.message = "Input cleared (Ctrl+C)".to_string();
                        edit.input = Some(input);
                    }
                    KeyCode::Down => {
                        if let Some(state) = suggest.as_mut() {
                            if !state.options.is_empty() {
                                state.selected = (state.selected + 1) % state.options.len();
                                edit.input = Some(input);
                                self.mode = UiMode::Edit(edit);
                                return Ok(false);
                            }
                        }
                        edit.input = Some(input);
                    }
                    KeyCode::Up => {
                        if let Some(state) = suggest.as_mut() {
                            if !state.options.is_empty() {
                                if state.selected == 0 {
                                    state.selected = state.options.len() - 1;
                                } else {
                                    state.selected -= 1;
                                }
                                edit.input = Some(input);
                                self.mode = UiMode::Edit(edit);
                                return Ok(false);
                            }
                        }
                        edit.input = Some(input);
                    }
                    KeyCode::Enter => {
                        if let Some(state) = suggest.as_ref() {
                            if !state.options.is_empty() {
                                let chosen = state.options[state.selected].clone();
                                apply_suggestion(value, state, &chosen);
                                *cursor = value.len();
                                *suggest = suggest_for_input(input.field, value, &edit.form.working_dir);
                                edit.input = Some(input);
                                self.mode = UiMode::Edit(edit);
                                return Ok(false);
                            }
                        }
                        edit.apply_input(input.field, value.clone());
                    }
                    KeyCode::Esc => {
                        if suggest.is_some() {
                            *suggest = None;
                            edit.input = Some(input);
                        } else {
                            edit.message = "Input canceled".to_string();
                        }
                    }
                    KeyCode::Backspace => {
                        let removed_char = if *cursor > 0 && *cursor <= value.len() {
                            value.chars().nth(*cursor - 1)
                        } else {
                            None
                        };
                        if *cursor > 0 && *cursor <= value.len() {
                            value.remove(*cursor - 1);
                            *cursor -= 1;
                        }
                        if let Some(ch) = removed_char {
                            if should_cancel_suggest_on_delete(suggest.as_ref(), ch) {
                                *suggest = None;
                            } else {
                                *suggest = suggest_for_input(input.field, value, &edit.form.working_dir);
                            }
                        } else {
                            *suggest = suggest_for_input(input.field, value, &edit.form.working_dir);
                        }
                        edit.input = Some(input);
                    }
                    KeyCode::Left => {
                        if *cursor > 0 {
                            *cursor -= 1;
                        }
                        edit.input = Some(input);
                    }
                    KeyCode::Right => {
                        if *cursor < value.len() {
                            *cursor += 1;
                        }
                        edit.input = Some(input);
                    }
                    KeyCode::Char(c) => {
                        if *cursor <= value.len() {
                            value.insert(*cursor, c);
                            *cursor += 1;
                        }
                        *suggest = suggest_for_input(input.field, value, &edit.form.working_dir);
                        edit.input = Some(input);
                    }
                    _ => {
                        edit.input = Some(input);
                    }
                },
                InputKind::Select { options, selected } => match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        *selected = (*selected + 1) % options.len();
                        edit.input = Some(input);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if *selected == 0 {
                            *selected = options.len() - 1;
                        } else {
                            *selected -= 1;
                        }
                        edit.input = Some(input);
                    }
                    KeyCode::Enter => {
                        edit.apply_input(input.field, options[*selected].clone());
                    }
                    KeyCode::Esc => {
                        edit.message = "Selection canceled".to_string();
                    }
                    _ => {
                        edit.input = Some(input);
                    }
                },
            }
            self.mode = UiMode::Edit(edit);
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => edit.next_field(),
            KeyCode::Char('k') | KeyCode::Up => edit.prev_field(),
            KeyCode::Enter => edit.activate_field(),
            KeyCode::Char('s') => match edit.to_job() {
                Ok(job) => {
                    write_job(paths, &job)?;
                    self.reload(paths)?;
                    self.selected = self
                        .jobs
                        .iter()
                        .position(|j| j.id == job.id)
                        .unwrap_or(self.selected);
                    self.mode = UiMode::List;
                    self.message = format!("Saved job {}", job.id);
                    return Ok(false);
                }
                Err(err) => {
                    edit.message = format!("Save failed: {err:#}");
                }
            },
            KeyCode::Char('q') | KeyCode::Esc => {
                if edit.dirty {
                    self.mode = UiMode::ConfirmDiscard {
                        edit: Box::new(edit),
                    };
                    return Ok(false);
                }
                self.mode = UiMode::List;
                self.message = "Back to list".to_string();
                return Ok(false);
            }
            _ => {}
        }

        self.mode = UiMode::Edit(edit);
        Ok(false)
    }
}

impl EditState {
    fn new(form: JobForm, msg: &str) -> Self {
        Self {
            form,
            selected: 0,
            dirty: false,
            input: None,
            message: msg.to_string(),
        }
    }

    fn fields(&self) -> Vec<EditField> {
        let mut fields = vec![EditField::Name, EditField::Enabled, EditField::ScheduleKind];
        match self.form.schedule_kind {
            ScheduleKind::Cron => fields.push(EditField::CronExpression),
            ScheduleKind::Simple => {
                fields.push(EditField::Repeat);
                match self.form.repeat {
                    Repeat::Daily => fields.push(EditField::Time),
                    Repeat::Weekly => {
                        fields.push(EditField::Weekday);
                        fields.push(EditField::Time);
                    }
                    Repeat::Monthly => {
                        fields.push(EditField::Day);
                        fields.push(EditField::Time);
                    }
                    Repeat::EveryMinute => {}
                    Repeat::Once => fields.push(EditField::OnceAt),
                }
            }
        }
        fields.extend([
            EditField::WorkingDir,
            EditField::Program,
            EditField::Args,
            EditField::EnvJson,
            EditField::Timeout,
        ]);
        fields
    }

    fn next_field(&mut self) {
        let fields = self.fields();
        if fields.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + 1) % fields.len();
    }

    fn prev_field(&mut self) {
        let fields = self.fields();
        if fields.is_empty() {
            self.selected = 0;
            return;
        }
        if self.selected == 0 {
            self.selected = fields.len() - 1;
        } else {
            self.selected -= 1;
        }
    }

    fn selected_field(&self) -> Option<EditField> {
        self.fields().get(self.selected).copied()
    }

    fn activate_field(&mut self) {
        let Some(field) = self.selected_field() else {
            return;
        };

        match field {
            EditField::Enabled => {
                self.form.enabled = !self.form.enabled;
                self.dirty = true;
                self.message = format!("enabled={}", self.form.enabled);
            }
            EditField::ScheduleKind => {
                self.form.schedule_kind = match self.form.schedule_kind {
                    ScheduleKind::Cron => ScheduleKind::Simple,
                    ScheduleKind::Simple => ScheduleKind::Cron,
                };
                self.dirty = true;
                self.selected = 0;
                self.message = "schedule type changed".to_string();
            }
            EditField::Repeat => {
                let options = vec![
                    "daily".to_string(),
                    "weekly".to_string(),
                    "monthly".to_string(),
                    "everyminute".to_string(),
                    "once".to_string(),
                ];
                let current = options
                    .iter()
                    .position(|v| v == repeat_label(&self.form.repeat))
                    .unwrap_or(0);
                self.input = Some(InputState {
                    field,
                    kind: InputKind::Select {
                        options,
                        selected: current,
                    },
                });
                self.message = "Select repeat with j/k, Enter apply".to_string();
            }
            _ => {
                let value = self.field_value(field);
                let cursor = value.len();
                let suggest = suggest_for_input(field, &value, &self.form.working_dir);
                self.input = Some(InputState {
                    field,
                    kind: InputKind::Text {
                        value,
                        cursor,
                        suggest,
                    },
                });
                self.message = "Editing field... Enter=apply Esc=cancel".to_string();
            }
        }
    }

    fn apply_input(&mut self, field: EditField, value: String) {
        match field {
            EditField::Name => self.form.name = value,
            EditField::CronExpression => self.form.cron_expression = value,
            EditField::Time => self.form.time = value,
            EditField::Weekday => {
                if let Ok(v) = value.parse::<u8>() {
                    self.form.weekday = v;
                }
            }
            EditField::Day => {
                if let Ok(v) = value.parse::<u8>() {
                    self.form.day = v;
                }
            }
            EditField::OnceAt => self.form.once_at = value,
            EditField::Program => self.form.program = value,
            EditField::Args => self.form.args = value,
            EditField::WorkingDir => self.form.working_dir = value,
            EditField::EnvJson => self.form.env_json = value,
            EditField::Timeout => self.form.timeout_seconds = value,
            EditField::Repeat => {
                self.form.repeat = parse_repeat(&value);
            }
            EditField::Enabled | EditField::ScheduleKind => {}
        }
        self.input = None;
        self.dirty = true;
        self.message = "Field updated".to_string();
    }

    fn field_value(&self, field: EditField) -> String {
        match field {
            EditField::Name => self.form.name.clone(),
            EditField::Enabled => self.form.enabled.to_string(),
            EditField::ScheduleKind => match self.form.schedule_kind {
                ScheduleKind::Cron => "cron".to_string(),
                ScheduleKind::Simple => "simple".to_string(),
            },
            EditField::CronExpression => self.form.cron_expression.clone(),
            EditField::Repeat => repeat_label(&self.form.repeat).to_string(),
            EditField::Time => self.form.time.clone(),
            EditField::Weekday => self.form.weekday.to_string(),
            EditField::Day => self.form.day.to_string(),
            EditField::OnceAt => self.form.once_at.clone(),
            EditField::Program => self.form.program.clone(),
            EditField::Args => self.form.args.clone(),
            EditField::WorkingDir => self.form.working_dir.clone(),
            EditField::EnvJson => self.form.env_json.clone(),
            EditField::Timeout => self.form.timeout_seconds.clone(),
        }
    }

    fn to_job(&self) -> Result<JobConfig> {
        let timeout_seconds: u64 = self
            .form
            .timeout_seconds
            .trim()
            .parse()
            .context("timeout_seconds must be number")?;
        let env: HashMap<String, String> = if self.form.env_json.trim().is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&self.form.env_json).context("env_json must be JSON object")?
        };

        let schedule = match self.form.schedule_kind {
            ScheduleKind::Cron => ScheduleConfig::Cron {
                expression: self.form.cron_expression.trim().to_string(),
            },
            ScheduleKind::Simple => {
                let repeat = self.form.repeat.clone();
                let (time, weekday, day, once_at) = match repeat {
                    Repeat::Daily => (Some(self.form.time.trim().to_string()), None, None, None),
                    Repeat::Weekly => (
                        Some(self.form.time.trim().to_string()),
                        Some(self.form.weekday),
                        None,
                        None,
                    ),
                    Repeat::Monthly => (
                        Some(self.form.time.trim().to_string()),
                        None,
                        Some(self.form.day),
                        None,
                    ),
                    Repeat::EveryMinute => (None, None, None, None),
                    Repeat::Once => (None, None, None, Some(self.form.once_at.trim().to_string())),
                };
                ScheduleConfig::Simple {
                    repeat,
                    time,
                    weekday,
                    day,
                    once_at,
                }
            }
        };

        let job = JobConfig {
            id: self.form.id.clone(),
            name: self.form.name.trim().to_string(),
            enabled: self.form.enabled,
            schedule,
            command: CommandConfig {
                program: self.form.program.trim().to_string(),
                args: split_args(&self.form.args),
                working_dir: if self.form.working_dir.trim().is_empty() {
                    None
                } else {
                    Some(self.form.working_dir.trim().to_string())
                },
                env,
            },
            timeout_seconds,
        };

        validate_candidate(&job)?;
        Ok(job)
    }
}

impl Clone for EditState {
    fn clone(&self) -> Self {
        Self {
            form: self.form.clone(),
            selected: self.selected,
            dirty: self.dirty,
            input: self.input.clone(),
            message: self.message.clone(),
        }
    }
}

impl JobForm {
    fn new(id: String) -> Self {
        Self {
            id,
            name: String::new(),
            enabled: false,
            schedule_kind: ScheduleKind::Simple,
            cron_expression: "0 2 * * *".to_string(),
            repeat: Repeat::Daily,
            time: "09:00".to_string(),
            weekday: 1,
            day: 1,
            once_at: Local::now().format("%Y-%m-%d %H:%M").to_string(),
            program: String::new(),
            args: String::new(),
            working_dir: String::new(),
            env_json: "{}".to_string(),
            timeout_seconds: "3600".to_string(),
        }
    }

    fn from_job(job: &JobConfig) -> Self {
        let (schedule_kind, cron_expression, repeat, time, weekday, day, once_at) = match &job.schedule {
            ScheduleConfig::Cron { expression } => (
                ScheduleKind::Cron,
                expression.clone(),
                Repeat::Daily,
                "09:00".to_string(),
                1,
                1,
                Local::now().format("%Y-%m-%d %H:%M").to_string(),
            ),
            ScheduleConfig::Simple {
                repeat,
                time,
                weekday,
                day,
                once_at,
            } => (
                ScheduleKind::Simple,
                "0 2 * * *".to_string(),
                repeat.clone(),
                time.clone().unwrap_or_else(|| "09:00".to_string()),
                weekday.unwrap_or(1),
                day.unwrap_or(1),
                once_at
                    .clone()
                    .unwrap_or_else(|| Local::now().format("%Y-%m-%d %H:%M").to_string()),
            ),
        };

        Self {
            id: job.id.clone(),
            name: job.name.clone(),
            enabled: job.enabled,
            schedule_kind,
            cron_expression,
            repeat,
            time,
            weekday,
            day,
            once_at,
            program: job.command.program.clone(),
            args: job.command.args.join(" "),
            working_dir: job.command.working_dir.clone().unwrap_or_default(),
            env_json: serde_json::to_string(&job.command.env).unwrap_or_else(|_| "{}".to_string()),
            timeout_seconds: job.timeout_seconds.to_string(),
        }
    }
}

fn render(frame: &mut Frame<'_>, ui: &UiState) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(8), Constraint::Length(4)])
        .split(frame.area());

    let daemon_text = match ui.daemon_pid {
        Some(pid) => format!("daemon: running(pid={pid})"),
        None => "daemon: stopped".to_string(),
    };
    let title = match &ui.mode {
        UiMode::List => format!("Macrond TUI - Jobs | {daemon_text}"),
        UiMode::Edit(_) => format!("Macrond TUI - Edit Job | {daemon_text}"),
        UiMode::ConfirmDelete { .. } => format!("Macrond TUI - Confirm Delete | {daemon_text}"),
        UiMode::ConfirmDiscard { .. } => format!("Macrond TUI - Confirm Discard | {daemon_text}"),
    };
    frame.render_widget(Paragraph::new(title), root[0]);

    match &ui.mode {
        UiMode::List => render_list(frame, root[1], ui),
        UiMode::Edit(edit) => render_edit(frame, root[1], edit),
        UiMode::ConfirmDelete { job_id } => {
            let p = Paragraph::new(format!("Delete job '{job_id}' ?\nPress y to confirm, n/Esc to cancel."))
                .block(Block::default().title("Confirm").borders(Borders::ALL));
            frame.render_widget(p, root[1]);
        }
        UiMode::ConfirmDiscard { .. } => {
            let p = Paragraph::new("Discard unsaved changes and return to list?\nPress y to discard, n/Esc to continue editing.")
                .block(Block::default().title("Confirm").borders(Borders::ALL));
            frame.render_widget(p, root[1]);
        }
    }

    let help = match &ui.mode {
        UiMode::List => {
            "h/Left:focus jobs  l/Right:focus history  j/k:move  a:add  e/Enter:edit  d:delete  s:toggle job  t:test job  S:start daemon  X:stop daemon  r:refresh  q:quit\nHistory focus: Enter shows selected full line in Status."
        }
        UiMode::Edit(edit) => {
            if edit.input.is_some() {
                "Input mode: type text  Ctrl+C:clear  Enter:apply  Backspace:delete  Esc:cancel\nEditor: j/k:move field  s:save  q/Esc:back"
            } else {
                "Editor: j/k:move field  Enter:edit/toggle  s:save  q/Esc:back\nRepeat options: daily/weekly/monthly/everyminute/once"
            }
        }
        UiMode::ConfirmDelete { .. } | UiMode::ConfirmDiscard { .. } => {
            "Confirm mode: y:yes  n:no  Esc:cancel\n"
        }
    };

    let footer = Paragraph::new(format!("{}\nStatus: {}", help, ui.message))
        .block(Block::default().title("Help").borders(Borders::ALL));
    frame.render_widget(footer, root[2]);
}

fn render_list(frame: &mut Frame<'_>, area: ratatui::layout::Rect, ui: &UiState) {
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let mut state = ListState::default().with_selected(Some(ui.selected));
    let job_items: Vec<ListItem<'_>> = if ui.jobs.is_empty() {
        vec![ListItem::new("No jobs. Press 'a' to create one.")]
    } else {
        ui.jobs
            .iter()
            .map(|job| {
                let schedule = scheduler::schedule_label(job);
                ListItem::new(format!(
                    "[{}] {} ({}) {}",
                    if job.enabled { "on" } else { "  " },
                    job.id,
                    job.name,
                    schedule
                ))
            })
            .collect()
    };

    let jobs_block = if ui.focus == ListFocus::Jobs {
        Block::default()
            .title("Jobs (focused)")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
    } else {
        Block::default().title("Jobs").borders(Borders::ALL)
    };
    let jobs = List::new(job_items)
        .block(jobs_block)
        .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
        .highlight_symbol(" > ");
    frame.render_stateful_widget(jobs, body[0], &mut state);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(body[1]);

    let mut history_state = ListState::default().with_selected(Some(ui.history_selected));
    let run_items: Vec<ListItem<'_>> = if ui.history_runs.is_empty() {
        vec![ListItem::new("No history log lines.")]
    } else {
        ui.history_runs
            .iter()
            .take(100)
            .map(|line| ListItem::new(line.clone()))
            .collect()
    };
    let history_block = if ui.focus == ListFocus::History {
        Block::default()
            .title("History Runs (focused)")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
    } else {
        Block::default().title("History Runs").borders(Borders::ALL)
    };
    let runs = List::new(run_items)
        .block(history_block)
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol(" > ");
    frame.render_stateful_widget(runs, right[0], &mut history_state);

    let detail = ui
        .history_runs
        .get(ui.history_selected)
        .cloned()
        .unwrap_or_else(|| "No history line selected".to_string());
    let detail_widget = Paragraph::new(detail)
        .block(Block::default().title("History Detail").borders(Borders::ALL))
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(detail_widget, right[1]);
}

fn render_edit(frame: &mut Frame<'_>, area: ratatui::layout::Rect, edit: &EditState) {
    let inner_width = area.width.saturating_sub(2);
    let content_width = inner_width.saturating_sub(3);
    let wrap_width = content_width.max(1) as usize;
    let fields = edit.fields();
    let selected = if fields.is_empty() {
        0
    } else {
        1 + edit.selected.min(fields.len().saturating_sub(1))
    };
    let mut state = ListState::default().with_selected(Some(selected));

    let mut items = Vec::new();
    items.push(ListItem::new(wrap_field_text("id (auto)", &edit.form.id, wrap_width)));

    for field in fields {
        let label = field_label(field);
        let value = edit.field_value(field);
        items.push(ListItem::new(wrap_field_text(label, &value, wrap_width)));
    }

    let editor = List::new(items)
        .block(
            Block::default()
                .title("Job Editor")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol(" > ");

    frame.render_stateful_widget(editor, area, &mut state);

    if let Some(input) = &edit.input {
        match &input.kind {
            InputKind::Text {
                value,
                cursor,
                suggest,
            } => {
                let popup_width = area.width.saturating_mul(80).saturating_div(100).max(10);
                let inner_width = popup_width.saturating_sub(2).max(1) as usize;
                let (text, cursor_pos) = wrap_input_text(field_label(input.field), value, *cursor, inner_width);
                let content_lines = text.lines.len().max(2);
                let popup_height = (content_lines + 2).min(area.height as usize).max(4) as u16;
                let popup = centered_rect_with_width(popup_width, popup_height, area);

                let widget = Paragraph::new(text)
                    .wrap(ratatui::widgets::Wrap { trim: false })
                    .block(Block::default().title("Input").borders(Borders::ALL));
                frame.render_widget(widget, popup);
                if let Some((cursor_x, cursor_y)) = cursor_pos {
                    frame.set_cursor_position((
                        popup.x.saturating_add(1).saturating_add(cursor_x),
                        popup.y.saturating_add(1).saturating_add(cursor_y),
                    ));
                }

                if let Some(state) = suggest {
                    render_suggest_list(frame, area, popup, state);
                }
            }
            InputKind::Select { options, selected } => {
                let mut lines = vec![format!("Select {}", field_label(input.field))];
                for (idx, opt) in options.iter().enumerate() {
                    if idx == *selected {
                        lines.push(format!("> {}", opt));
                    } else {
                        lines.push(format!("  {}", opt));
                    }
                }
                let select_popup = centered_rect(60, 9, area);
                let widget = Paragraph::new(lines.join("\n"))
                    .block(Block::default().title("Select").borders(Borders::ALL));
                frame.render_widget(widget, select_popup);
            }
        }
    }
}

fn wrap_field_text(label: &str, value: &str, width: usize) -> Text<'static> {
    let prefix = format!("{label}: ");
    let indent = " ".repeat(prefix.len());
    if value.is_empty() {
        return Text::from(vec![Line::from(prefix)]);
    }

    let first_width = width.saturating_sub(prefix.len()).max(1);
    let rest_width = width.saturating_sub(indent.len()).max(1);
    let (first, rest) = split_at_chars(value, first_width);
    let mut lines = Vec::new();
    lines.push(Line::from(format!("{prefix}{first}")));
    for chunk in split_chunks(&rest, rest_width) {
        lines.push(Line::from(format!("{indent}{chunk}")));
    }
    Text::from(lines)
}

fn wrap_input_text(label: &str, value: &str, cursor: usize, width: usize) -> (Text<'static>, Option<(u16, u16)>) {
    let title = format!("Editing {label}");
    let hint = if label == "program" {
        "Ctrl+C clear  @ browse files".to_string()
    } else {
        "Ctrl+C clear".to_string()
    };
    let prefix = "> ".to_string();
    let indent = "  ".to_string();
    let first_width = width.saturating_sub(prefix.len()).max(1);
    let rest_width = width.saturating_sub(indent.len()).max(1);

    let cursor_chars = value.get(0..cursor).unwrap_or(value).chars().count();
    let (first, rest) = split_at_chars(value, first_width);
    let mut lines = Vec::new();
    lines.push(Line::from(title));
    lines.push(Line::from(hint));
    lines.push(Line::from(format!("{prefix}{first}")));
    for chunk in split_chunks(&rest, rest_width) {
        lines.push(Line::from(format!("{indent}{chunk}")));
    }

    let (cursor_line, cursor_col) = if cursor_chars <= first_width {
        (2, prefix.len() + cursor_chars)
    } else {
        let remaining = cursor_chars - first_width;
        let line_offset = remaining / rest_width + 1;
        let col = indent.len() + (remaining % rest_width);
        (2 + line_offset, col)
    };

    let cursor_pos = Some((cursor_col as u16, cursor_line as u16));
    (Text::from(lines), cursor_pos)
}

fn suggest_for_input(field: EditField, value: &str, working_dir: &str) -> Option<SuggestState> {
    match field {
        EditField::WorkingDir => working_dir_suggest(value),
        EditField::Program => program_path_suggest(value, working_dir),
        _ => None,
    }
}

fn working_dir_suggest(value: &str) -> Option<SuggestState> {
    if !value.starts_with('/') {
        return None;
    }

    let (base, prefix) = match value.rfind('/') {
        Some(idx) => (value[..=idx].to_string(), value[idx + 1..].to_string()),
        None => ("/".to_string(), value.to_string()),
    };
    let dir = Path::new(&base);
    if !dir.is_dir() {
        return None;
    }

    let mut options = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if !prefix.is_empty() && !name.starts_with(&prefix) {
                continue;
            }
            options.push(name.to_string());
        }
    }

    if options.is_empty() {
        return None;
    }
    options.sort();
    Some(SuggestState {
        options,
        selected: 0,
        kind: SuggestKind::WorkingDir { base },
    })
}

fn program_path_suggest(value: &str, working_dir: &str) -> Option<SuggestState> {
    let at_pos = value.rfind('@')?;
    let after_at = &value[at_pos + 1..];
    let base_dir = if working_dir.trim().is_empty() {
        Path::new(".")
    } else {
        Path::new(working_dir)
    };
    if !base_dir.is_dir() {
        return None;
    }

    let search_root = base_dir.to_path_buf();
    let mut options = Vec::new();
    let mut count = 0usize;
    list_files_recursive(&search_root, &search_root, &mut options, &mut count, 300);
    let query = after_at.to_lowercase();
    options.retain(|path| {
        if !is_program_candidate(path) {
            return false;
        }
        if query.is_empty() {
            return true;
        }
        path.to_lowercase().contains(&query)
    });
    if options.is_empty() {
        return None;
    }
    options.sort();

    Some(SuggestState {
        options,
        selected: 0,
        kind: SuggestKind::ProgramPath {
            replace_start: at_pos,
            replace_end: at_pos + 1 + after_at.len(),
        },
    })
}

fn is_program_candidate(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext.is_empty() {
        return true;
    }
    if matches!(
        ext.as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "svg"
            | "ico"
            | "json"
            | "txt"
            | "md"
            | "csv"
            | "tsv"
            | "log"
            | "lock"
            | "map"
            | "pdf"
            | "zip"
            | "gz"
            | "tar"
            | "rar"
            | "7z"
    ) {
        return false;
    }
    matches!(
        ext.as_str(),
        "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "py"
            | "rb"
            | "pl"
            | "php"
            | "js"
            | "ts"
            | "jsx"
            | "tsx"
            | "lua"
            | "go"
            | "rs"
            | "swift"
            | "ps1"
            | "cmd"
            | "bat"
    )
}

fn list_files_recursive(
    root: &Path,
    current: &Path,
    out: &mut Vec<String>,
    count: &mut usize,
    limit: usize,
) {
    if *count >= limit {
        return;
    }
    let entries = match fs::read_dir(current) {
        Ok(v) => v,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *count >= limit {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            list_files_recursive(root, &path, out, count, limit);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel = rel.to_string_lossy().replace('\\', "/");
                out.push(rel);
                *count += 1;
            }
        }
    }
}

fn apply_suggestion(value: &mut String, state: &SuggestState, chosen: &str) {
    match &state.kind {
        SuggestKind::WorkingDir { base } => {
            *value = format!("{}{}/", base, chosen);
        }
        SuggestKind::ProgramPath {
            replace_start,
            replace_end,
        } => {
            let start = (*replace_start).min(value.len());
            let end = (*replace_end).min(value.len());
            let mut out = String::new();
            out.push_str(&value[..start]);
            out.push_str(chosen);
            out.push_str(&value[end..]);
            *value = out;
        }
    }
}

fn should_cancel_suggest_on_delete(suggest: Option<&SuggestState>, ch: char) -> bool {
    match suggest.map(|s| &s.kind) {
        Some(SuggestKind::WorkingDir { .. }) => ch == '/',
        Some(SuggestKind::ProgramPath { .. }) => ch == '@',
        None => false,
    }
}

fn render_suggest_list(
    frame: &mut Frame<'_>,
    area: ratatui::layout::Rect,
    popup: ratatui::layout::Rect,
    state: &SuggestState,
) {
    if state.options.is_empty() {
        return;
    }
    let max_visible = 8usize;
    let (visible, selected) = visible_options(&state.options, state.selected, max_visible);
    let list_height = (visible.len() + 2).max(3) as u16;
    let rect = dropdown_rect(popup, area, list_height);

    let items: Vec<ListItem<'_>> = visible.into_iter().map(ListItem::new).collect();
    let mut list_state = ListState::default().with_selected(Some(selected));
    let widget = List::new(items)
        .block(Block::default().title("Dirs").borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .highlight_symbol(" > ");
    frame.render_stateful_widget(widget, rect, &mut list_state);
}

fn visible_options(options: &[String], selected: usize, max_visible: usize) -> (Vec<String>, usize) {
    if options.len() <= max_visible {
        return (options.to_vec(), selected);
    }
    let half = max_visible / 2;
    let mut start = selected.saturating_sub(half);
    let end = (start + max_visible).min(options.len());
    if end - start < max_visible {
        start = end.saturating_sub(max_visible);
    }
    let slice = options[start..end].to_vec();
    let selected = selected.saturating_sub(start);
    (slice, selected)
}

fn dropdown_rect(
    popup: ratatui::layout::Rect,
    area: ratatui::layout::Rect,
    height: u16,
) -> ratatui::layout::Rect {
    let height = height.min(area.height).max(3);
    let below_space = area.y + area.height - (popup.y + popup.height);
    let above_space = popup.y.saturating_sub(area.y);
    let y = if below_space >= height {
        popup.y + popup.height
    } else if above_space >= height {
        popup.y.saturating_sub(height)
    } else {
        popup.y + popup.height
    };

    let mut rect = ratatui::layout::Rect {
        x: popup.x,
        y,
        width: popup.width,
        height,
    };

    if rect.y + rect.height > area.y + area.height {
        rect.y = area.y + area.height - rect.height;
    }
    rect
}

fn split_at_chars(s: &str, count: usize) -> (String, String) {
    let mut iter = s.chars();
    let head: String = iter.by_ref().take(count).collect();
    let tail: String = iter.collect();
    (head, tail)
}

fn split_chunks(s: &str, width: usize) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in s.chars() {
        if current.chars().count() >= width {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn field_label(field: EditField) -> &'static str {
    match field {
        EditField::Name => "name",
        EditField::Enabled => "enabled (Enter toggle)",
        EditField::ScheduleKind => "schedule_type (Enter toggle)",
        EditField::CronExpression => "cron_expression",
        EditField::Repeat => "repeat",
        EditField::Time => "time (HH:MM)",
        EditField::Weekday => "weekday (1-7)",
        EditField::Day => "day (1-31)",
        EditField::OnceAt => "once_at (YYYY-MM-DD HH:MM)",
        EditField::Program => "program",
        EditField::Args => "args",
        EditField::WorkingDir => "working_dir",
        EditField::EnvJson => "env_json",
        EditField::Timeout => "timeout_seconds",
    }
}

fn repeat_label(repeat: &Repeat) -> &'static str {
    match repeat {
        Repeat::Daily => "daily",
        Repeat::Weekly => "weekly",
        Repeat::Monthly => "monthly",
        Repeat::EveryMinute => "everyminute",
        Repeat::Once => "once",
    }
}

fn parse_repeat(s: &str) -> Repeat {
    match s {
        "weekly" => Repeat::Weekly,
        "monthly" => Repeat::Monthly,
        "everyminute" => Repeat::EveryMinute,
        "once" => Repeat::Once,
        _ => Repeat::Daily,
    }
}

fn split_args(s: &str) -> Vec<String> {
    if s.trim().is_empty() {
        Vec::new()
    } else {
        s.split_whitespace().map(|v| v.to_string()).collect()
    }
}

fn centered_rect(percent_x: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let width = area.width.saturating_mul(percent_x).saturating_div(100);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    ratatui::layout::Rect {
        x,
        y,
        width,
        height,
    }
}

fn centered_rect_with_width(width: u16, height: u16, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let width = width.min(area.width).max(3);
    let height = height.min(area.height).max(3);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    ratatui::layout::Rect {
        x,
        y,
        width,
        height,
    }
}

fn generate_job_id() -> String {
    format!("job-{}", Local::now().format("%Y%m%d%H%M%S%3f"))
}

fn write_job(paths: &AppPaths, job: &JobConfig) -> Result<()> {
    let path = job_file_path(&paths.jobs_dir, &job.id);
    fs::write(path, serde_json::to_vec_pretty(job)?)?;
    Ok(())
}

fn load_job_by_id(jobs_dir: &Path, job_id: &str) -> Result<JobConfig> {
    let path = job_file_path(jobs_dir, job_id);
    if !path.exists() {
        bail!("job file not found: {}", path.display());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn set_job_enabled(paths: &AppPaths, job_id: &str, enabled: bool) -> Result<()> {
    let mut job = load_job_by_id(&paths.jobs_dir, job_id)?;
    job.enabled = enabled;
    write_job(paths, &job)?;
    Ok(())
}

fn run_test(paths: &AppPaths, job_id: &str) -> Result<String> {
    let exe = std::env::current_exe()?;
    let output = StdCommand::new(exe)
        .arg("--base-dir")
        .arg(&paths.base_dir)
        .arg("run")
        .arg(job_id)
        .env("EZCRON_FORCE_INLINE", "1")
        .output()?;
    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if out.is_empty() {
            Ok(format!("Test finished for {job_id}"))
        } else {
            Ok(format!("Test result: {out}"))
        }
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(format!("Test failed for {job_id}: {err}"))
    }
}

fn daemon_command(paths: &AppPaths, cmd: &str) -> Result<String> {
    let exe = std::env::current_exe()?;
    let output = StdCommand::new(exe)
        .arg("--base-dir")
        .arg(&paths.base_dir)
        .arg(cmd)
        .output()?;
    if output.status.success() {
        let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if out.is_empty() {
            Ok(format!("daemon {cmd} done"))
        } else {
            Ok(out)
        }
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Ok(format!("daemon {cmd} failed: {err}"))
    }
}

fn validate_candidate(job: &JobConfig) -> Result<()> {
    let raw = serde_json::to_string(job)?;
    let parsed: JobConfig = serde_json::from_str(&raw)?;
    let dir = std::env::temp_dir().join(format!("macrond-validate-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", parsed.id));
    fs::write(&path, serde_json::to_vec_pretty(&parsed)?)?;
    let _ = config::load_jobs(&dir)?;
    fs::remove_file(path)?;
    fs::remove_dir_all(dir)?;
    Ok(())
}

fn job_file_path(jobs_dir: &Path, job_id: &str) -> std::path::PathBuf {
    jobs_dir.join(format!("{job_id}.json"))
}

fn load_history_runs(logs_dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(logs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if name.starts_with("job-") && name.ends_with(".log") {
            files.push(path);
        }
    }
    files.sort();
    let Some(latest) = files.last() else {
        return Ok(Vec::new());
    };

    let file = fs::File::open(latest)?;
    let reader = BufReader::new(file);
    let mut lines: Vec<String> = reader.lines().collect::<std::result::Result<Vec<_>, _>>()?;
    let start = lines.len().saturating_sub(100);
    lines = lines[start..].to_vec();
    lines.reverse();
    Ok(lines)
}

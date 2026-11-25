//! The Settings window (`SettingsDialog.tsx`): a page list beside the
//! chosen page, one window at a time. The Tauri app's was a dialog; here
//! it is a second native window (⌘W or Escape closes it).
//!
//! Every input is bound to a key ([`SettingsWindow::type_text`]): the
//! numbers of a form are its draft until Save, and Settings › Network's
//! whole page is one draft (`pm_view::settings::network`) with Revert and
//! Save. When the engine's saved value changes (a save, another window, an
//! agent), the form's fields are filled again from it, as the Tauri pages
//! remounted on a new value.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, FocusHandle, Focusable, Global, Render, ScrollHandle, Subscription,
    Task, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, px, size,
};
use pm_model::{DevicesSnapshot, McpStatus, NetworkSettings, StorageStatus};
use pm_view::settings::{self as logic, network};

use super::pages;
use crate::bridge::keys::{self, IntegrationCaches, RulesCaches, SettingsCaches};
use crate::bridge::stream::Latest;
use crate::bridge::{self, mutations};
use crate::features::clipboard::{self, report_error};
use crate::features::rules::{self, Target};
use crate::workspace::Workspace;

pub const DEFAULT_SIZE: (f32, f32) = (880., 680.);
pub const MIN_SIZE: (f32, f32) = (720., 480.);
pub const NAV_WIDTH: f32 = 168.;
/// The MCP page reads the audit log again this often (calls keep arriving
/// while agents work).
pub const AUDIT_POLL: Duration = Duration::from_secs(2);

/// The forms whose fields are filled from a saved value, with their keys.
const FORMS: [(&str, &[&str]); 6] = [
    ("proxy", &["proxy.port"]),
    (
        "storage",
        &["storage.max_flows", "storage.max_disk_mb", "storage.max_ws_messages"],
    ),
    ("autosave", &["autosave.folder", "autosave.interval", "autosave.keep"]),
    ("bp", &["adv.bp"]),
    ("scripts", &["adv.traffic_ms", "adv.view_ms"]),
    ("pool", &["adv.pool_idle", "adv.pool_max", "adv.connect"]),
];

/// Settings › Network's draft.
#[derive(Clone, Debug)]
pub struct NetworkDraft {
    /// What the engine saved (passwords gone, `has_password` set).
    pub base: NetworkSettings,
    pub draft: NetworkSettings,
    /// Fields whose text isn't a value (a port), by key.
    pub field_errors: BTreeMap<String, String>,
}

impl NetworkDraft {
    pub fn dirty(&self) -> bool {
        self.draft != self.base
    }
}

/// The open Settings window.
struct Open {
    window: AnyWindowHandle,
    view: WeakEntity<SettingsWindow>,
}

impl Global for Open {}

pub struct SettingsWindow {
    page: &'static str,
    pub(crate) caches: SettingsCaches,
    pub(crate) rules: RulesCaches,
    /// LAN access and the system proxy (G13's caches).
    pub(crate) integration: IntegrationCaches,
    pub(crate) storage: Entity<Latest<StorageStatus>>,
    pub(crate) devices: Entity<Latest<DevicesSnapshot>>,
    pub(crate) mcp: Entity<Latest<McpStatus>>,
    pub(crate) fields: HashMap<String, (Entity<InputState>, Subscription)>,
    /// An error under a form or section, by its id.
    pub(crate) errors: HashMap<&'static str, String>,
    /// A confirmation under a form ("Saved. …").
    pub(crate) notes: HashMap<&'static str, String>,
    /// Forms with a write in flight ("Restarting…", "Saving…").
    pub(crate) busy: HashSet<&'static str>,
    pub(crate) network: Option<NetworkDraft>,
    /// The saved value (as JSON) each form's fields were last filled from.
    bases: HashMap<&'static str, String>,
    pub(crate) confirm_regenerate: bool,
    pub(crate) audit_limit: u32,
    pub(crate) focus: FocusHandle,
    pub(crate) scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

/// Opens the window, or brings it forward, at `page` (`None`: as it was).
pub fn open_or_focus(page: Option<&'static str>, cx: &mut App) {
    if let Some((window, view)) = SettingsWindow::global(cx) {
        let _ = window.update(cx, |_, window, cx| {
            window.activate_window();
            if let Some(page) = page {
                view.update(cx, |w, cx| w.go(page, window, cx));
            }
        });
        return;
    }
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Settings".into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1)),
            cx,
        ))),
        window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
        app_id: Some(pm_app::BUNDLE_ID.into()),
        ..Default::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| SettingsWindow::new(page.unwrap_or(logic::DEFAULT_PAGE), window, cx));
        SettingsWindow::register(window.window_handle(), &view, cx);
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx))
    });
    match opened {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(err) => eprintln!("can't open the Settings window: {err}"),
    }
}

impl SettingsWindow {
    pub fn new(page: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let caches = keys::settings_caches(cx);
        let rules = keys::rules_caches(cx);
        let integration = keys::integration_caches(cx);
        let storage = bridge::storage(cx);
        let devices = bridge::devices(cx);
        let mcp = bridge::mcp(cx);
        let mut subscriptions = vec![
            cx.observe(&storage, |_, _, cx| cx.notify()),
            cx.observe(&devices, |_, _, cx| cx.notify()),
            cx.observe(&mcp, |_, _, cx| cx.notify()),
            cx.observe_in(&caches.proxy, window, |w, _, window, cx| w.sync(window, cx)),
            cx.observe_in(&caches.network, window, |w, _, window, cx| w.sync(window, cx)),
            cx.observe_in(&caches.storage, window, |w, _, window, cx| w.sync(window, cx)),
            cx.observe_in(&caches.autosave, window, |w, _, window, cx| w.sync(window, cx)),
            cx.observe_in(&caches.scripts, window, |w, _, window, cx| w.sync(window, cx)),
            cx.observe_in(&rules.settings, window, |w, _, window, cx| w.sync(window, cx)),
        ];
        macro_rules! notify_on {
            ($($e:expr),* $(,)?) => { $( subscriptions.push(cx.observe(&$e, |_, _, cx| cx.notify())); )* };
        }
        notify_on!(
            caches.recording,
            caches.proto_schemas,
            caches.proto_mappings,
            caches.openapi_specs,
            caches.mcp_token,
            caches.mcp_audit,
            caches.updates,
            integration.access,
            integration.system_proxy,
            caches.terminal_env,
            rules.mock_sources,
        );
        if let Some(ws) = Workspace::global(cx) {
            subscriptions.push(cx.observe(&ws, |_, _, cx| cx.notify()));
        }
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(AUDIT_POLL).await;
                let polled = this.update(cx, |w, cx| {
                    if w.page == "mcp" {
                        w.caches.mcp_audit.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
                    }
                });
                if polled.is_err() {
                    break;
                }
            }
        });
        let mut w = SettingsWindow {
            page: logic::page_or_default(page),
            caches,
            rules,
            integration,
            storage,
            devices,
            mcp,
            fields: HashMap::new(),
            errors: HashMap::new(),
            notes: HashMap::new(),
            busy: HashSet::new(),
            network: None,
            bases: HashMap::new(),
            confirm_regenerate: false,
            audit_limit: logic::AUDIT_PAGE,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
            _poll: poll,
        };
        w.sync(window, cx);
        w
    }

    /// Makes `view` the Settings window the actions and the dev remote act
    /// on (tests open theirs directly).
    pub fn register(window: AnyWindowHandle, view: &Entity<SettingsWindow>, cx: &mut App) {
        cx.set_global(Open {
            window,
            view: view.downgrade(),
        });
    }

    /// The open window and its view.
    pub fn global(cx: &App) -> Option<(AnyWindowHandle, Entity<SettingsWindow>)> {
        let open = cx.try_global::<Open>()?;
        Some((open.window, open.view.upgrade()?))
    }

    /// Runs `f` on the open window's view, in its window. Deferred: an
    /// app-level action handler runs while its window (maybe this one) is
    /// still dispatching the action.
    pub fn update_global(
        cx: &mut App,
        f: impl FnOnce(&mut SettingsWindow, &mut Window, &mut Context<SettingsWindow>) + 'static,
    ) {
        cx.defer(move |cx| {
            let Some((window, view)) = SettingsWindow::global(cx) else {
                return;
            };
            let _ = window.update(cx, |_, window, cx| view.update(cx, |w, cx| f(w, window, cx)));
        });
    }

    pub fn page(&self) -> &'static str {
        self.page
    }

    /// Shows a page.
    pub fn go(&mut self, page: &str, _: &mut Window, cx: &mut Context<Self>) {
        let page = logic::page_or_default(page);
        if page != self.page {
            self.page = page;
            self.confirm_regenerate = false;
            self.scroll.set_offset(Default::default());
        }
        cx.notify();
    }

    pub fn error(&self, form: &str) -> Option<&str> {
        self.errors.get(form).map(String::as_str)
    }

    pub fn note(&self, form: &str) -> Option<&str> {
        self.notes.get(form).map(String::as_str)
    }

    pub fn is_busy(&self, form: &str) -> bool {
        self.busy.contains(form)
    }

    pub fn network_draft(&self) -> Option<&NetworkDraft> {
        self.network.as_ref()
    }

    pub fn audit_limit(&self) -> u32 {
        self.audit_limit
    }

    pub fn confirming_regenerate(&self) -> bool {
        self.confirm_regenerate
    }

    /// Reads every cache the pages show (a page reads its own as it
    /// renders; the dev remote and tests act before a frame is drawn).
    pub fn prefetch(&mut self, cx: &mut Context<Self>) {
        let c = self.caches.clone();
        macro_rules! fetch {
            ($($e:expr),* $(,)?) => { $( $e.update(cx, |c, cx| { c.get(&(), cx); }); )* };
        }
        fetch!(
            c.proxy,
            c.network,
            c.storage,
            c.recording,
            c.autosave,
            c.proto_schemas,
            c.proto_mappings,
            c.openapi_specs,
            c.scripts,
            c.mcp_token,
            c.updates,
            self.integration.access,
            self.integration.system_proxy,
            c.terminal_env,
            self.rules.settings,
            self.rules.mock_sources,
        );
        let limit = self.audit_limit;
        c.mcp_audit.update(cx, |c, cx| {
            c.get(&limit, cx);
        });
    }

    /// Whether the forms' saved values are in (the network draft too).
    pub fn loaded(&self, cx: &App) -> bool {
        self.network.is_some()
            && self.caches.proxy.read(cx).peek(&()).is_some()
            && self.caches.storage.read(cx).peek(&()).is_some()
            && self.caches.autosave.read(cx).peek(&()).is_some()
    }

    pub fn any_busy(&self) -> bool {
        !self.busy.is_empty()
    }

    /// The keys of the inputs made so far, sorted (the dev remote, tests).
    pub fn field_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.fields.keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn errors_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.errors).unwrap_or_default()
    }

    pub fn notes_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.notes).unwrap_or_default()
    }

    pub fn busy_json(&self) -> serde_json::Value {
        let mut busy: Vec<&str> = self.busy.iter().copied().collect();
        busy.sort_unstable();
        serde_json::to_value(busy).unwrap_or_default()
    }

    // ---- Fields ----

    /// What field `key` shows when it is made: the saved value (or the
    /// network draft's).
    pub fn saved_text(&self, key: &str, cx: &App) -> String {
        let proxy = || self.caches.proxy.read(cx).peek(&()).copied();
        let storage = || self.caches.storage.read(cx).peek(&()).copied();
        let autosave = || self.caches.autosave.read(cx).peek(&()).cloned();
        let scripts = || self.caches.scripts.read(cx).peek(&()).copied();
        let text = match key {
            "proxy.port" => proxy().map(|p| p.port.to_string()),
            "storage.max_flows" => storage().map(|s| s.max_flows.to_string()),
            "storage.max_disk_mb" => storage().map(|s| s.max_disk_mb.to_string()),
            "storage.max_ws_messages" => storage().map(|s| s.max_ws_messages.to_string()),
            "autosave.folder" => autosave().map(|a| a.folder),
            "autosave.interval" => autosave().map(|a| a.interval_minutes.to_string()),
            "autosave.keep" => autosave().map(|a| a.keep.to_string()),
            "adv.bp" => self
                .rules
                .settings
                .read(cx)
                .peek(&())
                .map(|s| s.breakpoint_timeout_secs.to_string()),
            "adv.traffic_ms" => scripts().map(|s| s.traffic_timeout_ms.to_string()),
            "adv.view_ms" => scripts().map(|s| s.view_timeout_ms.to_string()),
            "adv.pool_idle" => proxy().map(|p| p.pool_idle_secs.to_string()),
            "adv.pool_max" => proxy().map(|p| p.pool_max_idle_per_host.to_string()),
            "adv.connect" => proxy().map(|p| p.connect_timeout_secs.to_string()),
            _ => {
                if let Some(rest) = key.strip_prefix("net.") {
                    self.network.as_ref().and_then(|n| network::text(&n.draft, rest))
                } else if let Some(ip) = key.strip_prefix("client.") {
                    self.devices
                        .read(cx)
                        .value
                        .as_ref()
                        .and_then(|d| d.devices.iter().find(|d| d.ip == ip))
                        .and_then(|d| d.nickname.clone())
                } else {
                    None
                }
            }
        };
        text.unwrap_or_default()
    }

    /// A field's text as typed (or as it would start).
    pub fn text(&self, key: &str, cx: &App) -> String {
        match self.fields.get(key) {
            Some((state, _)) => state.read(cx).value().to_string(),
            None => self.saved_text(key, cx),
        }
    }

    /// The input bound to `key`, made on first use.
    pub(crate) fn field(
        &mut self,
        key: &str,
        placeholder: &str,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((state, _)) = self.fields.get(key) {
            return state.clone();
        }
        let text = self.saved_text(key, cx);
        let placeholder = placeholder.to_owned();
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .masked(masked)
                .default_value(text)
        });
        let owned = key.to_owned();
        let sub = cx.subscribe_in(&state, window, move |this, state, e: &InputEvent, window, cx| match e {
            InputEvent::Change => {
                let value = state.read(cx).value().to_string();
                this.typed(&owned, &value, cx);
            }
            InputEvent::PressEnter { .. } => this.enter(&owned, window, cx),
            _ => {}
        });
        self.fields.insert(key.to_owned(), (state.clone(), sub));
        state
    }

    /// Types `value` into field `key` (the dev remote, tests): as typing
    /// does, the change goes through the field.
    pub fn type_text(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.field(key, "", false, window, cx);
        state.update(cx, |s, cx| s.set_value(value.to_owned(), window, cx));
        // `set_value` may not report a change: apply it either way.
        self.typed(key, value, cx);
    }

    fn set_field(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((state, _)) = self.fields.get(key)
            && state.read(cx).value() != value
        {
            let state = state.clone();
            state.update(cx, |s, cx| s.set_value(value.to_owned(), window, cx));
        }
    }

    fn typed(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        if let Some(rest) = key.strip_prefix("net.")
            && let Some(n) = &mut self.network
        {
            match network::set_text(&mut n.draft, rest, value) {
                Ok(()) => {
                    n.field_errors.remove(rest);
                }
                Err(err) => {
                    n.field_errors.insert(rest.to_owned(), err);
                }
            }
        }
        cx.notify();
    }

    /// ↩ in a field: adds to its list, or saves its form.
    fn enter(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let press = if let Some(list) = key.strip_prefix("list.") {
            format!("list.{list}.add")
        } else {
            match key.split('.').next() {
                Some("proxy") => "proxy.apply".into(),
                Some("storage") => "storage.save".into(),
                Some("autosave") => "autosave.save".into(),
                Some("adv") => match key {
                    "adv.bp" => "adv.bp.save".into(),
                    "adv.traffic_ms" | "adv.view_ms" => "adv.scripts.save".into(),
                    _ => "adv.pool.save".into(),
                },
                _ => return,
            }
        };
        self.press(&press, window, cx);
    }

    /// Fills each form's fields again from what the engine saved, when that
    /// changed (a save, a push, another window).
    pub(crate) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (form, keys) in FORMS {
            let Some(base) = self.base_of(form, cx) else { continue };
            if self.bases.get(form) == Some(&base) {
                continue;
            }
            self.bases.insert(form, base);
            for key in keys {
                let text = self.saved_text(key, cx);
                self.set_field(key, &text, window, cx);
            }
        }
        if let Some(saved) = self.caches.network.read(cx).peek(&()).cloned()
            && self.network.as_ref().is_none_or(|n| n.base != saved)
        {
            self.network = Some(NetworkDraft {
                base: saved.clone(),
                draft: saved,
                field_errors: BTreeMap::new(),
            });
            self.sync_network_fields(window, cx);
        }
        cx.notify();
    }

    fn base_of(&self, form: &str, cx: &App) -> Option<String> {
        let json = |v: Option<String>| v;
        json(match form {
            "proxy" | "pool" => self
                .caches
                .proxy
                .read(cx)
                .peek(&())
                .map(|p| serde_json::to_string(p).unwrap_or_default()),
            "storage" => self
                .caches
                .storage
                .read(cx)
                .peek(&())
                .map(|s| serde_json::to_string(s).unwrap_or_default()),
            "autosave" => self
                .caches
                .autosave
                .read(cx)
                .peek(&())
                .map(|s| serde_json::to_string(s).unwrap_or_default()),
            "bp" => self
                .rules
                .settings
                .read(cx)
                .peek(&())
                .map(|s| serde_json::to_string(s).unwrap_or_default()),
            "scripts" => self
                .caches
                .scripts
                .read(cx)
                .peek(&())
                .map(|s| serde_json::to_string(s).unwrap_or_default()),
            _ => None,
        })
    }

    /// The network inputs follow the draft: rows moved or removed, a
    /// picker that changed which fields a row has.
    fn sync_network_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(n) = &self.network else { return };
        let keys: Vec<String> = self.fields.keys().filter(|k| k.starts_with("net.")).cloned().collect();
        let mut gone = Vec::new();
        let mut set = Vec::new();
        for key in keys {
            match network::text(&n.draft, &key["net.".len()..]) {
                Some(text) => set.push((key, text)),
                None => gone.push(key),
            }
        }
        for key in gone {
            self.fields.remove(&key);
        }
        for (key, text) in set {
            self.set_field(&key, &text, window, cx);
        }
        if let Some(n) = &mut self.network {
            n.field_errors.clear();
        }
    }

    fn reset_form(&mut self, form: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((_, keys)) = FORMS.iter().find(|(f, _)| *f == form) {
            for key in *keys {
                let text = self.saved_text(key, cx);
                self.set_field(key, &text, window, cx);
            }
        }
        cx.notify();
    }

    /// The numbers of a form, parsed: `Err` names the first that isn't one.
    pub fn numbers(&self, keys: &[(&str, u32, u32)], cx: &App) -> Result<Vec<u32>, String> {
        keys.iter()
            .map(|(key, min, max)| logic::parse_number(&self.text(key, cx), *min, *max))
            .collect()
    }

    // ---- Switches and pickers ----

    pub fn choose(&mut self, key: &str, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let on = value == "true";
        match key {
            "page" => return self.go(value, window, cx),
            "proxy.lan" => mutations::set_lan_access(on, |_| {}, cx),
            "proxy.interface" => mutations::set_lan_interface(value.to_owned(), cx),
            "proxy.sysproxy" => mutations::set_system_proxy(on, |_| {}, cx),
            "autosave.enabled" => {
                let mut next = match self.autosave_draft(cx) {
                    Ok(draft) => draft,
                    Err(_) => self.caches.autosave.read(cx).peek(&()).cloned().unwrap_or_default(),
                };
                next.enabled = on;
                self.errors.remove("autosave");
                mutations::set_autosave_settings(next, |err, cx| Self::fail("autosave", err, cx), cx);
            }
            "appearance" => {
                let appearance = match value {
                    "light" => crate::prefs::Appearance::Light,
                    "dark" => crate::prefs::Appearance::Dark,
                    _ => crate::prefs::Appearance::System,
                };
                crate::appearance::set(appearance, cx);
            }
            "mcp.enabled" | "mcp.read_only" => {
                let Some(mut settings) = self.mcp.read(cx).value.as_ref().map(|s| s.settings.clone()) else {
                    return;
                };
                if key == "mcp.enabled" {
                    settings.enabled = on;
                } else {
                    settings.read_only = on;
                }
                self.errors.remove("mcp");
                mutations::set_mcp_settings(settings, |err, cx| Self::fail("mcp", err, cx), cx);
            }
            "updates.launch" => {
                self.errors.remove("updates");
                mutations::set_update_settings(
                    pm_model::UpdateSettings { check_at_launch: on },
                    |err, cx| Self::fail("updates", err, cx),
                    cx,
                );
            }
            _ => {
                if let Some(rest) = key.strip_prefix("net.")
                    && let Some(n) = &mut self.network
                    && network::choose(&mut n.draft, rest, value)
                {
                    self.sync_network_fields(window, cx);
                }
            }
        }
        cx.notify();
    }

    /// A failed write's error, shown under its form.
    fn fail(form: &'static str, err: String, cx: &mut App) {
        SettingsWindow::update_now(cx, move |w, cx| {
            w.busy.remove(form);
            w.errors.insert(form, err);
            cx.notify();
        });
    }

    /// A G13 write's answer: its failure goes under `form` (a cancelled
    /// prompt changed nothing, so it says nothing).
    fn failed_with(form: &'static str) -> impl FnOnce(Result<(), mutations::Failure>, &mut App) + 'static {
        move |answer, cx| {
            if let Err(err) = answer
                && !err.cancelled
            {
                Self::fail(form, err.message, cx);
            }
        }
    }

    /// A write is over: its form isn't busy any more.
    fn done(form: &'static str, cx: &mut App) {
        SettingsWindow::update_now(cx, move |w, cx| {
            w.busy.remove(form);
            cx.notify();
        });
    }

    /// Runs `f` on the open view, now (a write's answer, not an action).
    fn update_now(cx: &mut App, f: impl FnOnce(&mut SettingsWindow, &mut Context<SettingsWindow>)) {
        if let Some((_, view)) = SettingsWindow::global(cx) {
            view.update(cx, f);
        }
    }

    fn autosave_draft(&self, cx: &App) -> Result<pm_model::AutoSaveSettings, String> {
        let saved = self.caches.autosave.read(cx).peek(&()).cloned().unwrap_or_default();
        let n = self.numbers(&[("autosave.interval", 1, 1440), ("autosave.keep", 1, 1000)], cx)?;
        Ok(pm_model::AutoSaveSettings {
            folder: self.text("autosave.folder", cx).trim().to_owned(),
            interval_minutes: n[0],
            keep: n[1],
            ..saved
        })
    }

    // ---- Buttons ----

    /// A button by id (see [`super::SettingsPress`]).
    pub fn press(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let (id, arg) = match id.split_once(':') {
            Some((id, arg)) => (id, Some(arg.to_owned())),
            None => (id, None),
        };
        let arg_owned = arg.clone().unwrap_or_default();
        match id {
            "proxy.apply" => self.apply_port(cx),
            "net.save" => self.save_network(cx),
            "net.revert" => {
                if let Some(n) = &mut self.network {
                    n.draft = n.base.clone();
                    n.field_errors.clear();
                }
                self.errors.remove("network");
                self.sync_network_fields(window, cx);
            }
            "storage.save" => self.save_limits(cx),
            "storage.revert" => self.reset_form("storage", window, cx),
            "storage.clear" => {
                if let Some(ws) = Workspace::global(cx) {
                    ws.update(cx, |w, cx| w.clear_flows(cx));
                }
            }
            "autosave.save" => match self.autosave_draft(cx) {
                Ok(next) => {
                    self.errors.remove("autosave");
                    mutations::set_autosave_settings(next, |err, cx| Self::fail("autosave", err, cx), cx);
                }
                Err(err) => {
                    self.errors.insert("autosave", err);
                }
            },
            "autosave.now" => mutations::autosave_now(cx),
            "autosave.choose" => super::pick_folder("Choose the auto-save folder", cx, |paths, cx| {
                let Some(path) = paths.into_iter().next() else { return };
                let path = path.display().to_string();
                SettingsWindow::update_global(cx, move |w, window, cx| {
                    w.type_text("autosave.folder", &path, window, cx)
                });
            }),
            "proto.import" => {
                super::pick_file("Import .proto files or a descriptor set", true, cx, |paths, cx| {
                    import_proto(paths, cx);
                });
            }
            "proto.remove" => {
                if let Ok(id) = arg_owned.parse() {
                    self.errors.remove("proto");
                    mutations::remove_proto(id, |err, cx| Self::fail("proto", err, cx), cx);
                }
            }
            "proto.unmap" => {
                let mappings: Vec<_> = self
                    .caches
                    .proto_mappings
                    .read(cx)
                    .peek(&())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|m| m.id != arg_owned)
                    .collect();
                self.errors.remove("proto");
                mutations::set_proto_mappings(mappings, |err, cx| Self::fail("proto", err, cx), cx);
            }
            "mock.file" => super::mock_from_file(cx),
            "mock.remove" => {
                self.errors.remove("mocking");
                mutations::remove_mock_source(arg_owned, |err, cx| Self::fail("mocking", err, cx), cx);
            }
            "openapi.import" => super::import_openapi(cx),
            "openapi.generate" => {
                if let Ok(id) = arg_owned.parse() {
                    self.errors.remove("mocking");
                    generate_mocks(id, |err, cx| Self::fail("mocking", err, cx), cx);
                }
            }
            "openapi.remove" => {
                if let Ok(id) = arg_owned.parse() {
                    self.errors.remove("mocking");
                    mutations::remove_openapi(id, |err, cx| Self::fail("mocking", err, cx), cx);
                }
            }
            "columns.reset" => reset_columns(cx),
            "adv.bp.save" => match self.numbers(&[("adv.bp", 0, u32::MAX)], cx) {
                Ok(n) => {
                    self.errors.remove("bp");
                    mutations::save_rule_settings(
                        pm_model::RuleSettings {
                            breakpoint_timeout_secs: n[0],
                        },
                        |err, cx| Self::fail("bp", err, cx),
                        cx,
                    );
                }
                Err(err) => {
                    self.errors.insert("bp", err);
                }
            },
            "adv.scripts.save" => {
                match self.numbers(&[("adv.traffic_ms", 10, 60_000), ("adv.view_ms", 10, 60_000)], cx) {
                    Ok(n) => {
                        self.errors.remove("scripts");
                        mutations::set_script_settings(
                            pm_model::ScriptSettings {
                                traffic_timeout_ms: n[0],
                                view_timeout_ms: n[1],
                            },
                            |err, cx| Self::fail("scripts", err, cx),
                            cx,
                        );
                    }
                    Err(err) => {
                        self.errors.insert("scripts", err);
                    }
                }
            }
            "adv.pool.save" => self.save_pool(cx),
            "mcp.regenerate" => self.confirm_regenerate = true,
            "mcp.regenerate.cancel" => self.confirm_regenerate = false,
            "mcp.regenerate.confirm" => {
                self.confirm_regenerate = false;
                mutations::regenerate_mcp_token(cx);
            }
            "mcp.audit.refresh" => {
                self.caches.mcp_audit.update(cx, |c, cx| c.invalidate(|_, _| true, cx));
            }
            "mcp.audit.clear" => mutations::clear_mcp_audit(cx),
            "mcp.audit.more" => {
                self.audit_limit = (self.audit_limit + logic::AUDIT_MORE).min(logic::AUDIT_MAX);
            }
            "updates.check" => crate::updater::check_now(cx),
            "client.allow" | "client.deny" => {
                let allow = id == "client.allow";
                let nickname = allow
                    .then(|| logic::normalized(&self.text(&format!("client.{arg_owned}"), cx)).map(str::to_owned))
                    .flatten();
                self.errors.remove("clients");
                mutations::decide_device(arg_owned, allow, nickname, Self::failed_with("clients"), cx);
            }
            "client.forget" => {
                self.errors.remove("clients");
                mutations::forget_device(arg_owned, Self::failed_with("clients"), cx);
            }
            "copy" => {
                if let Some(field) = self.copy_field(&arg_owned, cx) {
                    clipboard::copy(&field.value, Some(&field.what), cx);
                }
            }
            "page" => self.go(&arg_owned, window, cx),
            _ => {
                if let Some(rest) = id.strip_prefix("net.add.") {
                    if let Some(n) = &mut self.network {
                        network::add(&mut n.draft, rest);
                    }
                } else if let Some(rest) = id.strip_prefix("net.remove.") {
                    if let Some((section, i)) = rest.split_once('.')
                        && let Ok(i) = i.parse()
                        && let Some(n) = &mut self.network
                        && network::remove(&mut n.draft, section, i)
                    {
                        self.sync_network_fields(window, cx);
                    }
                } else if let Some(rest) = id.strip_prefix("net.choose.") {
                    let key = format!("net.{rest}");
                    super::pick_file("Choose the client certificate file", false, cx, move |paths, cx| {
                        let Some(path) = paths.into_iter().next() else { return };
                        let path = path.display().to_string();
                        SettingsWindow::update_global(cx, move |w, window, cx| w.type_text(&key, &path, window, cx));
                    });
                } else if let Some(list) = id.strip_prefix("list.") {
                    self.list_press(list, arg.as_deref(), window, cx);
                }
            }
        }
        cx.notify();
    }

    fn start(form: &'static str, cx: &mut App) {
        SettingsWindow::update_now(cx, move |w, cx| {
            w.errors.remove(form);
            w.busy.insert(form);
            cx.notify();
        });
    }

    fn apply_port(&mut self, cx: &mut Context<Self>) {
        let Some(saved) = self.caches.proxy.read(cx).peek(&()).copied() else {
            return;
        };
        match logic::parse_number(&self.text("proxy.port", cx), logic::PORT_MIN, logic::PORT_MAX) {
            Ok(port) => {
                self.errors.remove("proxy");
                self.busy.insert("proxy");
                mutations::set_proxy_settings(
                    pm_model::ProxySettings {
                        port: port as u16,
                        ..saved
                    },
                    |cx| Self::done("proxy", cx),
                    |err, cx| Self::fail("proxy", err, cx),
                    cx,
                );
            }
            Err(err) => {
                self.errors.insert("proxy", err);
            }
        }
    }

    fn save_pool(&mut self, cx: &mut Context<Self>) {
        let Some(saved) = self.caches.proxy.read(cx).peek(&()).copied() else {
            return;
        };
        match self.numbers(
            &[
                ("adv.pool_idle", 0, u32::MAX),
                ("adv.pool_max", 0, 1024),
                ("adv.connect", 1, 300),
            ],
            cx,
        ) {
            Ok(n) => {
                self.errors.remove("pool");
                self.busy.insert("pool");
                mutations::set_proxy_settings(
                    pm_model::ProxySettings {
                        pool_idle_secs: n[0],
                        pool_max_idle_per_host: n[1],
                        connect_timeout_secs: n[2],
                        ..saved
                    },
                    |cx| Self::done("pool", cx),
                    |err, cx| Self::fail("pool", err, cx),
                    cx,
                );
            }
            Err(err) => {
                self.errors.insert("pool", err);
            }
        }
    }

    fn save_limits(&mut self, cx: &mut Context<Self>) {
        let keys = logic::LIMITS.map(|(id, ..)| id);
        let fields: Vec<(String, u32, u32)> = keys.iter().map(|id| (format!("storage.{id}"), 0, u32::MAX)).collect();
        let refs: Vec<(&str, u32, u32)> = fields.iter().map(|(k, a, b)| (k.as_str(), *a, *b)).collect();
        self.notes.remove("storage");
        match self.numbers(&refs, cx) {
            Ok(n) => {
                self.errors.remove("storage");
                mutations::set_storage_settings(
                    pm_model::StorageSettings {
                        max_flows: n[0],
                        max_disk_mb: n[1],
                        max_ws_messages: n[2],
                    },
                    |cx| {
                        SettingsWindow::update_now(cx, |w, cx| {
                            w.notes
                                .insert("storage", "Saved. The capture was trimmed to the new limits.".into());
                            cx.notify();
                        })
                    },
                    |err, cx| Self::fail("storage", err, cx),
                    cx,
                );
            }
            Err(err) => {
                self.errors.insert("storage", err);
            }
        }
    }

    fn save_network(&mut self, cx: &mut Context<Self>) {
        let Some(n) = &self.network else { return };
        if !n.field_errors.is_empty() {
            return;
        }
        let draft = n.draft.clone();
        self.errors.remove("network");
        self.busy.insert("network");
        mutations::set_network_settings(
            draft,
            |cx| Self::done("network", cx),
            |err, cx| Self::fail("network", err, cx),
            cx,
        );
    }

    /// A list editor's Add or Remove: `list` is `<list id>.add` or
    /// `<list id>.remove` (with the item as `arg`).
    fn list_press(&mut self, list: &str, arg: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((list_id, op)) = list.rsplit_once('.') else {
            return;
        };
        let items = self.list_items(list_id, cx);
        let next = match (op, arg) {
            ("add", _) => {
                let field = format!("list.{list_id}");
                let draft = self.text(&field, cx);
                let Some(next) = logic::add_item(&items, &draft) else {
                    return;
                };
                self.set_field(&field, "", window, cx);
                next
            }
            ("remove", Some(item)) => logic::remove_item(&items, item),
            _ => return,
        };
        self.save_list(list_id, next, cx);
    }

    /// The items of a list editor: `ssl.<list>`, `rec.never`, `rec.only` or
    /// `net.bypass` (the network draft's).
    pub fn list_items(&self, list_id: &str, cx: &App) -> Vec<String> {
        match list_id.split_once('.') {
            Some(("ssl", name)) => Workspace::global(cx)
                .and_then(|ws| logic::ssl_list(&ws.read(cx).ssl, name).cloned())
                .unwrap_or_default(),
            Some(("rec", which)) => {
                let lists = self.caches.recording.read(cx).peek(&()).cloned().unwrap_or_default();
                if which == "only" { lists.only } else { lists.never }
            }
            Some(("net", "bypass")) => self
                .network
                .as_ref()
                .map(|n| n.draft.upstream.bypass.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn save_list(&mut self, list_id: &str, items: Vec<String>, cx: &mut Context<Self>) {
        match list_id.split_once('.') {
            Some(("ssl", name)) => {
                let Some(ssl) = Workspace::global(cx).map(|ws| ws.read(cx).ssl.clone()) else {
                    return;
                };
                if let Some(next) = logic::with_ssl_list(&ssl, name, items) {
                    self.errors.remove("ssl");
                    mutations::set_ssl_settings(next, |err, cx| Self::fail("ssl", err, cx), cx);
                }
            }
            Some(("rec", which)) => {
                let mut lists = self.caches.recording.read(cx).peek(&()).cloned().unwrap_or_default();
                if which == "only" {
                    lists.only = items;
                } else {
                    lists.never = items;
                }
                self.errors.remove("recording");
                mutations::set_recording_settings(lists, |err, cx| Self::fail("recording", err, cx), cx);
            }
            Some(("net", "bypass")) => {
                if let Some(n) = &mut self.network {
                    n.draft.upstream.bypass = items;
                }
            }
            _ => {}
        }
        cx.notify();
    }
}

/// Generate mocks: the rules are saved turned off; a toast says how many
/// and the Rules window opens on the first.
pub fn generate_mocks(id: u32, failed: impl FnOnce(String, &mut App) + 'static, cx: &mut App) {
    mutations::generate_openapi_mocks(
        id,
        |made, cx| {
            clipboard::info(&logic::generated_toast(made.len()), None, cx);
            if let Some(first) = made.first() {
                rules::open(Target::Rule(first.id.clone()), cx);
            }
        },
        failed,
        cx,
    );
}

/// Appearance › Reset columns: the table's layout back to its defaults
/// (header columns stay, as the header menu's item does).
pub fn reset_columns(cx: &mut App) {
    match crate::features::flow_table::first(cx) {
        Some(table) => {
            let grid = table.read(cx).grid().clone();
            grid.update(cx, |g, cx| g.reset_columns(cx));
        }
        None => crate::prefs::update(cx, |p| p.table.reset_columns()),
    }
    clipboard::info("Columns reset", None, cx);
}

/// Imports a protobuf schema (Settings › Protobuf's Import…, the dev
/// remote's `importProto`); the page shows "busy" meanwhile, and the error.
pub fn import_proto(paths: Vec<std::path::PathBuf>, cx: &mut App) {
    SettingsWindow::start("proto", cx);
    mutations::import_proto(
        paths,
        |_, cx| SettingsWindow::done("proto", cx),
        |err, cx| {
            if SettingsWindow::global(cx).is_some() {
                SettingsWindow::fail("proto", err, cx);
            } else {
                report_error("Couldn't import the schema", err, cx);
            }
        },
        cx,
    );
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        pages::render(self, window, cx)
    }
}

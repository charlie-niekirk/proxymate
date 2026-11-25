//! The shell chrome's actions (toolbar, sidebar, filter bar, footer):
//! the sidebar's scopes and SSL toggles, and the targets that belong to
//! later phases (the inspector's Comment…: G11, Rewrite…: G9), which say
//! so in a toast until those phases replace the handlers. G7's
//! `features::rules` handles [`OpenRule`]; G8's `features::compose`
//! handles [`Compose`]; G12's `features::settings` handles
//! [`OpenSettings`]; G13's `features::integration` handles
//! [`OpenDeviceSetup`] and [`OpenCertWizard`].

use gpui_kit::{App, SharedString};

use crate::features::clipboard::info;
use crate::features::documents::with_workspace;

/// Scopes the table to one app (`None`: all traffic).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ShowApp {
    pub app: Option<SharedString>,
}

/// Scopes the table to one host or `*.domain` (`None`: all traffic).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ShowHost {
    pub host: Option<SharedString>,
}

/// Toggles the search text between `text` and nothing (Pinned flows).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct ToggleText {
    pub text: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct AddSslBypass {
    pub host: SharedString,
}

#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetInsecureUpstream {
    pub host: SharedString,
    pub allow: bool,
}

/// "Don't decrypt HTTPS from <app>", by bundle id, else name.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct SetAppSslExcluded {
    pub app: SharedString,
    pub excluded: bool,
}

/// Settings › `page` (`features::settings`; empty: as it was).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct OpenSettings {
    pub page: SharedString,
}

/// The Rules window on one rule (`features::rules` handles it).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct OpenRule {
    pub id: SharedString,
}

/// The Comment… dialog for one flow (G11 builds it).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct OpenComment {
    pub store: pm_model::StoreId,
    pub id: pm_model::FlowId,
}

/// The Rewrite dialog for one flow (G9 builds it).
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = proxymate, no_json)]
pub struct RewriteFlow {
    pub store: pm_model::StoreId,
    pub id: pm_model::FlowId,
}

gpui_kit::actions!(proxymate, [OpenDeviceSetup, Compose, OpenCertWizard]);

pub fn register(cx: &mut App) {
    cx.on_action(|a: &ShowApp, cx| {
        let app = a.app.as_ref().map(ToString::to_string);
        with_workspace(cx, |w, cx| w.set_scope(None, app, cx));
    });
    cx.on_action(|a: &ShowHost, cx| {
        let host = a.host.as_ref().map(ToString::to_string);
        with_workspace(cx, |w, cx| w.set_scope(host, None, cx));
    });
    cx.on_action(|a: &ToggleText, cx| {
        let text = a.text.to_string();
        with_workspace(cx, |w, cx| {
            let next = if w.filter.text.trim() == text {
                String::new()
            } else {
                text
            };
            w.set_text(next, cx);
        });
    });
    cx.on_action(|a: &AddSslBypass, cx| {
        let host = a.host.to_string();
        with_workspace(cx, |w, cx| w.add_ssl_bypass(host, cx));
    });
    cx.on_action(|a: &SetInsecureUpstream, cx| {
        let (host, allow) = (a.host.to_string(), a.allow);
        with_workspace(cx, |w, cx| w.set_insecure_upstream(host, allow, cx));
    });
    cx.on_action(|a: &SetAppSslExcluded, cx| {
        let (app, excluded) = (a.app.to_string(), a.excluded);
        with_workspace(cx, |w, cx| w.set_app_ssl_excluded(app, excluded, cx));
    });
    cx.on_action(|_: &OpenComment, cx| {
        info("Comment", Some("The Comment dialog arrives in G11.".into()), cx);
    });
    cx.on_action(|_: &RewriteFlow, cx| {
        info("Rewrite", Some("The Rewrite dialog arrives in G9.".into()), cx);
    });
}

//! The root CA's surfaces (`CertWizard.tsx`, `CertificateMenu.tsx`,
//! `Toolbar.tsx` `TrustWarning`): the certificate wizard (the keychain and
//! a real handshake checked, Install & Trust, Remove, the Firefox and
//! terminal notes), the toolbar's Certificate popover (trust state,
//! details, copies, Regenerate) and its "Certificate not trusted" warning.
//! Every keychain change is a mutation; the caches hold what was read.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, AnyView, App, Context, Entity, FontWeight, Global, MouseButton, Role, Subscription, Window, div, px,
};
use pm_model::{CaInfo, KeychainTrust, TrustStatus};
use pm_theme::kit::TokensExt as _;
use pm_view::integration as view;

use super::ui;
use crate::bridge::keys::{IntegrationCaches, integration_caches};
use crate::bridge::mutations;
use crate::features::chrome::OpenCertWizard;
use crate::features::clipboard;
use crate::features::context_menu::Menu;
use crate::workspace::Workspace;

/// How long "Copied" stays on a copy button.
const COPIED_FOR: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Busy {
    Install,
    Uninstall,
}

/// The wizard's content: one per app, shown in a dialog of the main window.
pub struct CertWizard {
    caches: IntegrationCaches,
    pub busy: Option<Busy>,
    pub error: Option<String>,
    copied: bool,
    _subscriptions: Vec<Subscription>,
}

/// The wizard while its dialog is open.
#[derive(Default)]
struct WizardOpen(Option<Entity<CertWizard>>);

impl Global for WizardOpen {}

impl CertWizard {
    pub fn new(cx: &mut Context<Self>) -> CertWizard {
        let caches = integration_caches(cx);
        let subscriptions = vec![
            cx.observe(&caches.trust, |_, _, cx| cx.notify()),
            cx.observe(&caches.ca, |_, _, cx| cx.notify()),
        ];
        CertWizard {
            caches,
            busy: None,
            error: None,
            copied: false,
            _subscriptions: subscriptions,
        }
    }

    /// The wizard whose dialog is open, if it is.
    pub fn open_one(cx: &App) -> Option<Entity<CertWizard>> {
        cx.try_global::<WizardOpen>().and_then(|o| o.0.clone())
    }

    fn trust(&self, cx: &mut Context<Self>) -> Option<TrustStatus> {
        self.caches.trust.update(cx, |c, cx| c.get(&(), cx)).ready().cloned()
    }

    fn checking(&self, cx: &App) -> bool {
        self.caches.trust.read(cx).is_fetching(&())
    }

    /// Install & Trust (`install`) or Remove from keychain: macOS asks first.
    pub fn change(&mut self, kind: Busy, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(kind);
        self.error = None;
        cx.notify();
        let this = cx.weak_entity();
        let done = move |answer: Result<(), mutations::Failure>, cx: &mut App| {
            let _ = this.update(cx, |w, cx| {
                w.busy = None;
                if let Err(err) = answer {
                    w.error = Some(view::trust_error(err.cancelled, &err.message));
                }
                cx.notify();
            });
        };
        match kind {
            Busy::Install => mutations::install_ca_trust(done, cx),
            Busy::Uninstall => mutations::uninstall_ca_trust(done, cx),
        }
    }

    /// Check again: the keychain and a fresh handshake.
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.caches.trust.update(cx, |c, cx| c.retry(&(), cx));
        cx.notify();
    }

    fn copy_env(&mut self, cx: &mut Context<Self>) {
        let Some(env) = Workspace::global(cx).and_then(|w| w.read(cx).terminal_env()) else {
            return;
        };
        clipboard::copy_silent(&env, cx);
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |w, cx| {
                w.copied = false;
                cx.notify();
            });
        })
        .detach();
    }
}

/// Opens the wizard over the main window (once).
pub fn open_wizard(window: &mut Window, cx: &mut App) {
    if CertWizard::open_one(cx).is_some() {
        return;
    }
    let wizard = cx.new(CertWizard::new);
    cx.set_global(WizardOpen(Some(wizard.clone())));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .w(px(480.))
            .on_close(|_, _, cx| cx.set_global(WizardOpen(None)))
            .child(wizard.clone())
    });
}

/// Closes the wizard's dialog (Done).
pub fn close_wizard(window: &mut Window, cx: &mut App) {
    if CertWizard::open_one(cx).is_some() {
        cx.set_global(WizardOpen(None));
        window.close_dialog(cx);
    }
}

impl Render for CertWizard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let trust = self.trust(cx);
        let ca = self.caches.ca.update(cx, |c, cx| c.get(&(), cx)).ready().cloned();
        let checking = self.checking(cx);
        let installed = trust.as_ref().is_some_and(|t| t.keychain == KeychainTrust::Trusted);
        let verified = trust.as_ref().is_some_and(|t| t.verified);
        let unsupported = trust.as_ref().is_some_and(|t| t.keychain == KeychainTrust::Unsupported);
        let busy = self.busy;
        let [keychain, handshake] = view::wizard_checks(trust.as_ref());

        let header = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .id("cert-wizard-title")
                    .role(Role::Heading)
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(pm_widgets::icon(IconName::ShieldCheck, t.foreground.hsla(), 16.))
                    .child("Trust the Proxymate certificate"),
            )
            .child(div().text_color(t.muted_foreground.hsla()).child(
                "To decrypt HTTPS, apps on this Mac must trust Proxymate's root CA. It is added to your login \
                 keychain for your user only; macOS asks for your password or Touch ID.",
            ));

        let mut lines = vec![ui::check(&keychain, &t), ui::check(&handshake, &t)];
        if let Some(ca) = &ca {
            lines.push(
                div()
                    .pl(px(24.))
                    .font_family(ui::MONO)
                    .text_size(px(11.))
                    .text_color(t.muted_foreground.hsla())
                    .child(ca.common_name.clone())
                    .into_any_element(),
            );
        }
        let checks = ui::checklist(lines, &t);

        let status = if verified {
            div()
                .text_color(t.s_2xx.hsla())
                .child("Safari, Chrome and other apps that use the macOS trust store will now accept Proxymate's certificates.")
                .into_any_element()
        } else {
            ui::rich(
                &[
                    ("Click ", false),
                    ("Install & Trust", true),
                    (
                        ", then approve the macOS prompt. Proxymate then checks the result with a real handshake.",
                        false,
                    ),
                ],
                &t,
            )
        };

        let copied = self.copied;
        let notes = div()
            .id("cert-wizard-notes")
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(10.))
            .rounded(px(6.))
            .bg(t.muted.hsla())
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Firefox and command-line tools"),
            )
            .child(ui::rich(
                &[
                    ("Firefox keeps its own certificate store. Open ", false),
                    ("about:config", true),
                    (", set ", false),
                    ("security.enterprise_roots.enabled", true),
                    (" to ", false),
                    ("true", true),
                    (" and restart Firefox so it trusts the keychain.", false),
                ],
                &t,
            ))
            .child(div().text_color(t.muted_foreground.hsla()).child(
                "Node, Python, curl and friends read CA files instead. Copy the terminal environment and paste it \
                 into a shell.",
            ))
            .child(
                div().flex().child(
                    Button::new("cert-wizard-copy-env")
                        .outline()
                        .xsmall()
                        .icon(IconName::Copy)
                        .label(if copied { "Copied" } else { "Copy terminal env" })
                        .on_click(cx.listener(|w, _, _, cx| w.copy_env(cx))),
                ),
            );

        let remove = Button::new("uninstall-ca")
            .ghost()
            .small()
            .disabled(!installed || busy.is_some())
            .loading(busy == Some(Busy::Uninstall))
            .icon(IconName::Trash)
            .label("Remove from keychain")
            .on_click(cx.listener(|w, _, _, cx| w.change(Busy::Uninstall, cx)));
        let check_again = Button::new("cert-wizard-check")
            .outline()
            .small()
            .disabled(busy.is_some() || checking)
            .loading(checking)
            .icon(IconName::RefreshCw)
            .label("Check again")
            .on_click(cx.listener(|w, _, _, cx| w.recheck(cx)));
        let main = if verified {
            Button::new("cert-wizard-done")
                .primary()
                .small()
                .label("Done")
                .on_click(|_, window, cx| close_wizard(window, cx))
        } else {
            Button::new("install-ca")
                .primary()
                .small()
                .disabled(busy.is_some() || unsupported)
                .loading(busy == Some(Busy::Install))
                .label(if busy == Some(Busy::Install) {
                    "Waiting for macOS…"
                } else {
                    "Install & Trust"
                })
                .on_click(cx.listener(|w, _, _, cx| w.change(Busy::Install, cx)))
        };
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(remove)
            .child(div().flex().gap(px(8.)).child(check_again).child(main));

        div()
            .id("cert-wizard")
            .flex()
            .flex_col()
            .gap(px(12.))
            .text_size(px(12.))
            .role(Role::Dialog)
            .aria_label("Trust the Proxymate certificate")
            .child(header)
            .child(checks)
            .child(status)
            .child(notes)
            .when_some(self.error.clone(), |d, e| d.child(ui::error(e, &t)))
            .child(footer)
            .test_support()
    }
}

// ---- The toolbar's Certificate popover ----

/// The popover's content: trust, the CA's details and Regenerate.
pub struct CertPanel {
    caches: IntegrationCaches,
    pub confirming: bool,
    /// The "remove the old CA's trust" switch, once touched (else it
    /// follows whether this Mac trusts the CA).
    untrust: Option<bool>,
    pub busy: bool,
    pub error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl CertPanel {
    pub fn new(cx: &mut Context<Self>) -> CertPanel {
        let caches = integration_caches(cx);
        let subscriptions = vec![
            cx.observe(&caches.trust, |_, _, cx| cx.notify()),
            cx.observe(&caches.ca, |_, _, cx| cx.notify()),
        ];
        CertPanel {
            caches,
            confirming: false,
            untrust: None,
            busy: false,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn remove_old(&self, trust: Option<&TrustStatus>) -> bool {
        self.untrust
            .unwrap_or_else(|| trust.is_some_and(|t| t.keychain == KeychainTrust::Trusted))
    }

    /// Regenerate… asks first; Replace CA does it.
    pub fn regenerate(&mut self, cx: &mut Context<Self>) {
        if !self.confirming {
            self.confirming = true;
            return cx.notify();
        }
        let trust = self.caches.trust.read(cx).peek(&()).cloned();
        let remove_old = self.remove_old(trust.as_ref());
        self.confirming = false;
        self.error = None;
        self.busy = true;
        cx.notify();
        let this = cx.weak_entity();
        mutations::regenerate_ca(
            remove_old,
            move |answer, cx| {
                let _ = this.update(cx, |p, cx| {
                    p.busy = false;
                    p.error = answer.err().map(|e| e.message);
                    cx.notify();
                });
            },
            cx,
        );
    }

    pub fn set_untrust(&mut self, on: bool, cx: &mut Context<Self>) {
        self.untrust = Some(on);
        cx.notify();
    }

    /// The popover closed: a pending confirmation is dropped.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.confirming = false;
        cx.notify();
    }
}

/// The CA menu (right-click on the details): every value to copy.
pub fn ca_menu(ca: &CaInfo, listening: Option<&str>) -> Menu {
    Menu::new()
        .copy("Copy common name", ca.common_name.clone(), "common name")
        .copy("Copy expiry date", ca.not_after.clone(), "expiry date")
        .copy("Copy SHA-256", view::colon_hex(&ca.sha256), "fingerprint")
        .copy("Copy SHA-256 without colons", ca.sha256.clone(), "fingerprint")
        .copy("Copy PEM file path", ca.cert_path.clone(), "path")
        .copy(
            "Copy curl example",
            view::curl_example(listening, &ca.cert_path),
            "command",
        )
}

fn listening(cx: &App) -> Option<String> {
    Workspace::global(cx).and_then(|w| w.read(cx).proxy.listening.clone())
}

/// The CA menu as the app would show it now.
pub fn current_ca_menu(cx: &mut App) -> Option<Menu> {
    let ca = integration_caches(cx).ca.read(cx).peek(&()).cloned()?;
    Some(ca_menu(&ca, listening(cx).as_deref()))
}

impl Render for CertPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let trust = self.caches.trust.update(cx, |c, cx| c.get(&(), cx)).ready().cloned();
        let ca = self.caches.ca.update(cx, |c, cx| c.get(&(), cx)).ready().cloned();
        let verified = trust.as_ref().is_some_and(|t| t.verified);
        let listening = listening(cx);
        let row = || div().px(px(12.)).py(px(8.)).border_b_1().border_color(t.border.hsla());

        let head = row()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .id("certificate-panel-title")
                    .role(Role::Heading)
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("HTTPS decryption"),
            )
            .child(
                div()
                    .text_color(t.muted_foreground.hsla())
                    .child("Clients must trust this root CA to accept Proxymate's certificates."),
            );
        let state = row()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(if verified {
                        pm_widgets::icon(IconName::CircleCheck, t.s_2xx.hsla(), 14.)
                    } else {
                        pm_widgets::icon(IconName::CircleAlert, t.s_4xx.hsla(), 14.)
                    })
                    .child(if verified {
                        "Trusted by this Mac"
                    } else {
                        "Not trusted by this Mac yet"
                    }),
            )
            .child({
                let b = Button::new("open-cert-wizard")
                    .xsmall()
                    .label(if verified { "Manage…" } else { "Install & Trust…" })
                    .on_click(|_, window, cx| {
                        super::close_tool_popovers(cx);
                        window.dispatch_action(Box::new(OpenCertWizard), cx);
                    });
                if verified { b.outline() } else { b.primary() }
            });

        let details = ca.as_ref().map(|ca| {
            let curl = view::curl_example(listening.as_deref(), &ca.cert_path);
            let menu_ca = ca.clone();
            let menu_listening = listening.clone();
            div()
                .id("ca-details")
                .px(px(12.))
                .py(px(8.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(ui::field("Root CA", 72., ca.common_name.clone(), &t))
                .child(ui::field(
                    "Valid until",
                    72.,
                    ca.not_after.get(..10).unwrap_or(&ca.not_after).to_owned(),
                    &t,
                ))
                .child(ui::field(
                    "SHA-256",
                    72.,
                    div()
                        .font_family(ui::MONO)
                        .text_size(px(11.))
                        .whitespace_normal()
                        .child(view::colon_hex(&ca.sha256)),
                    &t,
                ))
                .child(ui::copy_row(
                    "PEM file",
                    72.,
                    ui::copy_field(
                        "ca-pem",
                        ca.cert_path.clone(),
                        "Copy PEM file path",
                        "path",
                        true,
                        vec![],
                        &t,
                    ),
                    &t,
                ))
                .child(ui::copy_row(
                    "Try it",
                    72.,
                    ui::copy_field("ca-curl", curl, "Copy curl example", "command", true, vec![], &t),
                    &t,
                ))
                .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                    ca_menu(&menu_ca, menu_listening.as_deref()).show(e.position, window, cx);
                })
        });

        let remove_old = self.remove_old(trust.as_ref());
        let confirming = self.confirming;
        let regenerate = div()
            .px(px(12.))
            .py(px(8.))
            .border_t_1()
            .border_color(t.border.hsla())
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(div().text_color(t.muted_foreground.hsla()).child(if confirming {
                        "Every device must trust the new CA again."
                    } else {
                        "A new CA must be trusted again."
                    }))
                    .child({
                        let b = Button::new("regenerate-ca")
                            .xsmall()
                            .disabled(self.busy)
                            .loading(self.busy)
                            .icon(IconName::RefreshCw)
                            .label(if confirming { "Replace CA" } else { "Regenerate…" })
                            .on_click(cx.listener(|p, _, _, cx| p.regenerate(cx)));
                        if confirming { b.danger() } else { b.outline() }
                    }),
            )
            .when(confirming, |d| {
                d.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(ui::switch(
                            "untrust-old-ca",
                            Switch::new("untrust-old-ca-switch")
                                .small()
                                .checked(remove_old)
                                .accessibility_label("Remove trust for the old CA from this Mac")
                                .on_click(cx.listener(|p, on: &bool, _, cx| p.set_untrust(*on, cx))),
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child("Remove trust for the old CA from this Mac")
                                .child(
                                    div()
                                        .text_color(t.muted_foreground.hsla())
                                        .child("macOS asks for your password; other devices keep their copy."),
                                ),
                        ),
                )
            });

        div()
            .id("certificate-panel")
            .w(px(384.))
            .flex()
            .flex_col()
            .text_size(px(12.))
            .child(head)
            .child(state)
            .children(details)
            .child(regenerate)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .px(px(12.))
                        .py(px(8.))
                        .border_t_1()
                        .border_color(t.border.hsla())
                        .child(ui::error(e, &t)),
                )
            })
            .test_support()
    }
}

/// The toolbar's Certificate button and its popover.
pub struct CertificateTool {
    caches: IntegrationCaches,
    panel: Entity<CertPanel>,
    pub open: bool,
    _subscriptions: Vec<Subscription>,
}

pub fn certificate_tool(_: &mut Window, cx: &mut App) -> AnyView {
    let tool = cx.new(CertificateTool::new);
    super::track_tool(super::Tool::Certificate, &tool, cx);
    tool.into()
}

impl CertificateTool {
    pub fn new(cx: &mut Context<Self>) -> CertificateTool {
        let caches = integration_caches(cx);
        let subscriptions = vec![cx.observe(&caches.trust, |_, _, cx| cx.notify())];
        CertificateTool {
            caches,
            panel: cx.new(CertPanel::new),
            open: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn panel(&self) -> &Entity<CertPanel> {
        &self.panel
    }

    pub fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open == open {
            return;
        }
        self.open = open;
        if !open {
            self.panel.update(cx, |p, cx| p.reset(cx));
        }
        cx.notify();
    }
}

impl Render for CertificateTool {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let trust = self.caches.trust.update(cx, |c, cx| c.get(&(), cx)).ready().cloned();
        let verified = trust.as_ref().is_some_and(|t| t.verified);
        let (icon, colour) = match &trust {
            Some(_) if !verified => (IconName::ShieldAlert, t.foreground),
            _ if verified => (IconName::ShieldCheck, t.s_2xx),
            _ => (IconName::ShieldCheck, t.foreground),
        };
        let panel = self.panel.clone();
        let this = cx.weak_entity();
        Popover::new("certificate-popover")
            .anchor(Anchor::TopRight)
            .p_0()
            .open(self.open)
            .on_open_change(move |open, _, cx| {
                let _ = this.update(cx, |tool, cx| tool.set_open(*open, cx));
            })
            .trigger(
                Button::new("certificate-menu")
                    .ghost()
                    .small()
                    .accessibility_label(if trust.is_some() && !verified {
                        "Certificate: not trusted"
                    } else {
                        "Certificate"
                    })
                    .tooltip("Certificate")
                    .child(pm_widgets::icon(icon, colour.hsla(), 15.)),
            )
            .content(move |_, _, _| panel.clone())
    }
}

/// "Certificate not trusted": while listening and the CA isn't verified.
pub struct TrustWarning {
    caches: IntegrationCaches,
    _subscriptions: Vec<Subscription>,
}

pub fn trust_warning(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let caches = integration_caches(cx);
        let mut subscriptions = vec![cx.observe(&caches.trust, |_, _, cx| cx.notify())];
        if let Some(ws) = Workspace::global(cx) {
            subscriptions.push(cx.observe(&ws, |_, _, cx| cx.notify()));
        }
        TrustWarning {
            caches,
            _subscriptions: subscriptions,
        }
    })
    .into()
}

impl Render for TrustWarning {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = *cx.tokens();
        let listening = listening(cx).is_some();
        let trust = self.caches.trust.update(cx, |c, cx| c.get(&(), cx)).ready().cloned();
        if !view::trust_warning(listening, trust.as_ref()) {
            return div().into_any_element();
        }
        Button::new("trust-warning")
            .ghost()
            .small()
            .accessibility_label("Certificate not trusted")
            .tooltip("HTTPS traffic can't be decrypted yet · open the certificate wizard")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .text_color(t.s_4xx.hsla())
                    .child(pm_widgets::icon(IconName::ShieldAlert, t.s_4xx.hsla(), 15.))
                    .child("Certificate not trusted"),
            )
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenCertWizard), cx))
            .into_any_element()
    }
}

/// Shared by the panel's and the wizard's tests: a CA like the app's.
pub fn sample_ca() -> CaInfo {
    CaInfo {
        common_name: "Proxymate CA (Mac.local, 2026-11-21)".into(),
        cert_path: "/Users/me/Library/Application Support/Proxymate/ca/proxymate-ca.pem".into(),
        not_before: "2026-11-21T09:00:00Z".into(),
        not_after: "2036-11-19T09:00:00Z".into(),
        sha256: "5e8c0f6b2a7d4e1c9b3a8f0e6d2c4b1a7f9e3d5c8b0a2e4f6c1d3b5a7e9f0c2d".into(),
    }
}

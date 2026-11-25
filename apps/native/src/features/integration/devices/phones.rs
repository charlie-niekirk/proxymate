//! The iPhone and Android tabs (`DeviceSetup/PhoneTabs.tsx`): the setup
//! page's QR code, the steps with the Wi-Fi proxy's server and port to
//! copy, Android's debug-build snippets, and each allowed device's
//! progress.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Image, div, px};
use pm_model::{DeviceAccess, DevicesSnapshot};
use pm_theme::Tokens;
use pm_view::devices::{self as view, Progress};

use super::qr_element;
use crate::features::integration::ui;

/// The QR code's SVG when LAN access is on and there is a setup URL.
pub fn qr_parts(access: Option<&DeviceAccess>) -> Option<String> {
    let access = access.filter(|a| a.enabled && a.setup_url.is_some())?;
    access.qr_svg.clone()
}

/// The QR code and the setup URL, or why there is none yet.
fn qr_card(access: Option<&DeviceAccess>, qr: Option<Arc<Image>>, device: &str, t: &Tokens) -> AnyElement {
    let url = access.filter(|a| a.enabled).and_then(|a| a.setup_url.clone());
    let (Some(url), Some(qr)) = (url, qr) else {
        return div()
            .flex()
            .items_center()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(6.))
            .border_1()
            .border_dashed()
            .border_color(t.border.hsla())
            .bg(t.muted.hsla())
            .child(pm_widgets::icon(IconName::Wifi, t.muted_foreground.hsla(), 16.))
            .child(div().flex_1().child(ui::rich(
                &[
                    ("Turn on ", false),
                    ("Allow devices on your network", true),
                    (
                        &format!(" above so your {device} can reach this Mac. The QR code appears here."),
                        false,
                    ),
                ],
                t,
            )))
            .into_any_element();
    };
    div()
        .id("device-qr")
        .flex()
        .items_center()
        .gap(px(12.))
        .p(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(t.border.hsla())
        .child(qr_element(qr, format!("QR code for {url}"), t))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().text_color(t.muted_foreground.hsla()).child(format!(
                    "Scan with the {device}'s camera, or open this in its browser once its proxy is set:"
                )))
                .child(ui::copy_field(
                    "copy-setup-url",
                    url,
                    "Copy setup URL",
                    "setup URL",
                    false,
                    vec![],
                    t,
                ))
                .child(ui::rich(
                    &[
                        ("Through the proxy, ", false),
                        ("http://proxy.mate/", true),
                        (" works too.", false),
                    ],
                    t,
                )),
        )
        .test_support()
        .into_any_element()
}

/// Server and port to type into the phone's Wi-Fi proxy settings.
fn proxy_fields(access: Option<&DeviceAccess>, t: &Tokens) -> Option<AnyElement> {
    let (host, port) = view::split_proxy(access.and_then(|a| a.proxy.as_deref()))?;
    Some(
        div()
            .mt(px(4.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(ui::copy_row(
                "Server",
                48.,
                ui::copy_field("copy-proxy-server", host, "Copy server", "server", false, vec![], t),
                t,
            ))
            .child(ui::copy_row(
                "Port",
                48.,
                ui::copy_field("copy-proxy-port", port, "Copy port", "port", false, vec![], t),
                t,
            ))
            .into_any_element(),
    )
}

/// "Waiting for a device…", then each allowed device's progress.
fn remote_activity(snapshot: &DevicesSnapshot, device: &str, t: &Tokens) -> AnyElement {
    let active = view::active_devices(&snapshot.devices);
    let lines = if active.is_empty() {
        vec![ui::check(
            &view::progress_check(Progress::Waiting, &format!("your {device}"), 0),
            t,
        )]
    } else {
        active
            .iter()
            .map(|d| {
                let mut line = ui::check(&view::progress_check(view::device_progress(d), &d.label, d.flows), t);
                if d.flows == 0 && d.onboarding {
                    line = div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(line)
                        .child(
                            div()
                                .ml(px(24.))
                                .text_color(t.muted_foreground.hsla())
                                .child("Opened the setup page; no proxied traffic yet."),
                        )
                        .into_any_element();
                }
                line
            })
            .collect()
    };
    div()
        .id("remote-activity")
        .child(ui::checklist(lines, t))
        .into_any_element()
}

fn step(parts: &[(&str, bool)], extra: Option<AnyElement>, t: &Tokens) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(ui::rich(parts, t))
        .children(extra)
        .into_any_element()
}

pub fn iphone_tab(
    access: Option<&DeviceAccess>,
    snapshot: &DevicesSnapshot,
    qr: Option<Arc<Image>>,
    t: &Tokens,
) -> AnyElement {
    let steps = ui::steps(
        vec![
            step(&[("Join the same Wi-Fi network as this Mac.", false)], None, t),
            step(
                &[
                    ("Open ", false),
                    ("Settings › Wi-Fi", true),
                    (", tap ", false),
                    ("ⓘ", true),
                    (" next to the network, then ", false),
                    ("Configure Proxy › Manual", true),
                    (". Enter this server and port, and tap Save.", false),
                ],
                proxy_fields(access, t),
                t,
            ),
            step(
                &[
                    (
                        "Scan the QR code with the Camera (or open the setup URL in Safari). Proxymate asks on this \
                         Mac whether to allow the iPhone: click ",
                        false,
                    ),
                    ("Allow", true),
                    (".", false),
                ],
                None,
                t,
            ),
            step(
                &[
                    ("Tap ", false),
                    ("Download profile", true),
                    (" and allow the download, then open ", false),
                    ("Settings › Profile Downloaded", true),
                    (" and tap ", false),
                    ("Install", true),
                    (".", false),
                ],
                None,
                t,
            ),
            step(
                &[
                    ("Open ", false),
                    ("Settings › General › About › Certificate Trust Settings", true),
                    (
                        " and turn on full trust for the Proxymate CA. Without this step HTTPS still fails.",
                        false,
                    ),
                ],
                None,
                t,
            ),
        ],
        t,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(qr_card(access, qr, "iPhone", t))
        .child(ui::section(Some("Set up the iPhone"), vec![steps]))
        .child(ui::section(
            Some("Status"),
            vec![remote_activity(snapshot, "iPhone", t)],
        ))
        .child(ui::rich(
            &[
                (
                    "To stop, set the Wi-Fi proxy back to Off. Remove the profile in ",
                    false,
                ),
                ("Settings › General › VPN & Device Management", true),
                (".", false),
            ],
            t,
        ))
        .into_any_element()
}

pub fn android_tab(
    access: Option<&DeviceAccess>,
    snapshot: &DevicesSnapshot,
    qr: Option<Arc<Image>>,
    t: &Tokens,
) -> AnyElement {
    let steps = ui::steps(
        vec![
            step(&[("Join the same Wi-Fi network as this Mac.", false)], None, t),
            step(
                &[
                    ("Open ", false),
                    ("Settings › Network & internet › Internet", true),
                    (
                        ", tap the gear next to the network, then the pencil (Edit). Under ",
                        false,
                    ),
                    ("Advanced options › Proxy", true),
                    (" choose ", false),
                    ("Manual", true),
                    (", enter this hostname and port, and save.", false),
                ],
                proxy_fields(access, t),
                t,
            ),
            step(
                &[
                    (
                        "Scan the QR code (or open the setup URL in Chrome) and allow the device on this Mac. Tap ",
                        false,
                    ),
                    ("Download certificate", true),
                    (".", false),
                ],
                None,
                t,
            ),
            step(
                &[
                    ("Open ", false),
                    (
                        "Settings › Security & privacy › More security settings › Encryption & credentials › \
                         Install a certificate › CA certificate",
                        true,
                    ),
                    (", tap ", false),
                    ("Install anyway", true),
                    (" and pick ", false),
                    ("proxymate-ca.crt", true),
                    (" from Downloads.", false),
                ],
                None,
                t,
            ),
        ],
        t,
    );
    div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(qr_card(access, qr, "phone", t))
        .child(ui::section(Some("Set up the Android device"), vec![steps]))
        .child(ui::section(
            Some("Your app's debug builds"),
            vec![
                ui::rich(
                    &[
                        (
                            "Chrome trusts user CAs, but apps targeting Android 7 or later don't unless they opt in. \
                             Add this to your app as ",
                            false,
                        ),
                        ("res/xml/network_security_config.xml", true),
                        ("; it applies to debug builds only.", false),
                    ],
                    t,
                ),
                ui::copy_field(
                    "copy-network-security-config",
                    view::NETWORK_SECURITY_CONFIG.to_owned(),
                    "Copy network_security_config.xml",
                    "network_security_config.xml",
                    true,
                    vec![],
                    t,
                ),
                div()
                    .text_color(t.muted_foreground.hsla())
                    .child("Then point the manifest at it:")
                    .into_any_element(),
                ui::copy_field(
                    "copy-manifest-attribute",
                    view::MANIFEST_ATTRIBUTE.to_owned(),
                    "Copy manifest attribute",
                    "manifest attribute",
                    true,
                    vec![],
                    t,
                ),
            ],
        ))
        .child(ui::section(
            Some("Status"),
            vec![remote_activity(snapshot, "Android device", t)],
        ))
        .into_any_element()
}

/// Under a switched-off access bar.
pub fn no_lan_hint(t: &Tokens) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .text_color(t.muted_foreground.hsla())
        .child(pm_widgets::icon(IconName::Smartphone, t.muted_foreground.hsla(), 14.))
        .child("Phones need device access; the Simulator and emulator don't.")
        .into_any_element()
}

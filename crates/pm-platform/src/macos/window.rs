//! AppKit window chrome: sidebar vibrancy and the window number.

// One scoped exception to the workspace's `unsafe_code = "deny"`: the
// window handle hands us the gpui content view as a raw `NSView` pointer,
// and turning it into a reference is the only unsafe step. Everything after
// it goes through objc2's safe AppKit bindings on the main thread.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSUserInterfaceItemIdentification, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowOrderingMode,
};

use crate::PlatformError;

/// The view's identifier, so a second call doesn't stack another one.
const VIBRANCY_ID: &str = "proxymate-sidebar-vibrancy";

fn window_of(ns_view: NonNull<c_void>) -> Result<(MainThreadMarker, Retained<NSWindow>), PlatformError> {
    let mtm = MainThreadMarker::new().ok_or_else(|| PlatformError::Os("not on the main thread".into()))?;
    // SAFETY: an AppKit window handle's `ns_view` is a live NSView for as
    // long as the window is (the caller holds the window), and we are on
    // the main thread, where AppKit objects may be touched.
    let view: &NSView = unsafe { ns_view.cast::<NSView>().as_ref() };
    let window = view
        .window()
        .ok_or_else(|| PlatformError::Os("the view has no window".into()))?;
    Ok((mtm, window))
}

pub fn apply_sidebar_vibrancy(ns_view: NonNull<c_void>) -> Result<(), PlatformError> {
    let (mtm, window) = window_of(ns_view)?;
    let content = window
        .contentView()
        .ok_or_else(|| PlatformError::Os("the window has no content view".into()))?;
    let id = objc2_foundation::NSString::from_str(VIBRANCY_ID);
    if content
        .subviews()
        .iter()
        .any(|v| v.identifier().is_some_and(|i| *i == *id))
    {
        return Ok(());
    }
    let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), content.bounds());
    effect.setMaterial(NSVisualEffectMaterial::Sidebar);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect.setState(NSVisualEffectState::FollowsWindowActiveState);
    effect.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    effect.setIdentifier(Some(&id));
    // Below gpui's Metal view, which paints every pane opaque but the sidebar.
    content.addSubview_positioned_relativeTo(&effect, NSWindowOrderingMode::Below, None);
    Ok(())
}

pub fn window_number(ns_view: NonNull<c_void>) -> Result<i64, PlatformError> {
    let (_, window) = window_of(ns_view)?;
    Ok(window.windowNumber() as i64)
}

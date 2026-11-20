//! G0 spike (b): a PDFKit `PDFView` as a child NSView of a gpui window.
//! It checks z-order (a gpui-drawn popover over the native view), clipping
//! (the native view inside a smaller clipping container, as a scrolled pane
//! needs) and that the PDF renders, then saves a window capture and quits.
//!
//! `cargo run -p proxymate-native --features spikes --example spike_native_view -- <capture.png>`
//! (macOS; the capture needs Screen Recording permission). Results are in
//! `docs/PLAN.md` (Native track, G0 spikes).

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("spike_native_view: macOS only");
}

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(target_os = "macos")]
mod macos {
    // The spike reaches AppKit through the window's raw NSView pointer, as
    // pm-platform's vibrancy does; the production version lives there (G5).
    #![allow(unsafe_code)]

    use std::time::Duration;

    use gpui_kit::prelude::*;
    use gpui_kit::{
        App, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds, WindowOptions, div, point, px, rgb,
        size,
    };
    use objc2::{AnyThread as _, MainThreadOnly as _};
    use objc2_app_kit::NSView;
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL};
    use objc2_pdf_kit::{PDFDocument, PDFView};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const PDF: &str = "/System/Library/ProductDocuments/ProductGuides/ENERGY STAR.pdf";

    struct Spike;

    impl Render for Spike {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(rgb(0x202024))
                .text_color(rgb(0xffffff))
                .child(
                    div()
                        .absolute()
                        .top(px(40.))
                        .left(px(40.))
                        .child("PDFView at (40, 80, 480×400), unclipped"),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(40.))
                        .left(px(600.))
                        .child("PDFView at (600, 80, 480×400) in a 300×200 clip"),
                )
                // A gpui "popover" that overlaps both native views.
                .child(
                    div()
                        .absolute()
                        .top(px(200.))
                        .left(px(300.))
                        .w(px(520.))
                        .h(px(120.))
                        .bg(rgb(0xff2d55))
                        .rounded(px(8.))
                        .p(px(12.))
                        .child("gpui popover: drawn by gpui over the native views?"),
                )
        }
    }

    fn pdf_view(frame: NSRect, mtm: objc2::MainThreadMarker) -> objc2::rc::Retained<PDFView> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(PDF));
        // SAFETY: PDFKit calls on the main thread with live objects.
        unsafe {
            let view = PDFView::initWithFrame(PDFView::alloc(mtm), frame);
            let doc = PDFDocument::initWithURL(PDFDocument::alloc(), &url).expect("open the PDF");
            view.setDocument(Some(&doc));
            view.setAutoScales(true);
            view
        }
    }

    pub fn run() {
        let capture = std::env::args().nth(1);
        gpui_kit::application().run(move |cx: &mut App| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("spike (b)".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(16.), px(17.))),
                }),
                window_bounds: Some(WindowBounds::Windowed(gpui_kit::Bounds {
                    origin: point(px(80.), px(80.)),
                    size: size(px(1140.), px(560.)),
                })),
                ..Default::default()
            };
            let handle = cx
                .open_window(options, |window, cx| {
                    let mtm = objc2::MainThreadMarker::new().unwrap();
                    let RawWindowHandle::AppKit(h) = HasWindowHandle::window_handle(window).unwrap().as_raw() else {
                        unreachable!()
                    };
                    // SAFETY: the window's live content NSView, on the main thread.
                    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
                    let content = view.window().unwrap().contentView().unwrap();
                    let flipped = content.isFlipped();
                    let height = content.bounds().size.height;
                    let rect = |x: f64, y: f64, w: f64, h: f64| {
                        let y = if flipped { y } else { height - y - h };
                        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
                    };
                    eprintln!("content view flipped: {flipped}");
                    // Unclipped, straight on the content view (above gpui's Metal view).
                    content.addSubview(&pdf_view(rect(40., 80., 480., 400.), mtm));
                    // Clipped: a 300×200 container that clips a 480×400 PDFView.
                    let clip = NSView::initWithFrame(NSView::alloc(mtm), rect(600., 80., 300., 200.));
                    clip.setWantsLayer(true);
                    if let Some(layer) = clip.layer() {
                        layer.setMasksToBounds(true);
                    }
                    let inner = pdf_view(NSRect::new(NSPoint::new(0., 0.), NSSize::new(480., 400.)), mtm);
                    clip.addSubview(&inner);
                    content.addSubview(&clip);
                    let number = pm_platform::window::window_number(window).unwrap();
                    eprintln!("window number {number}");
                    cx.new(|cx| gpui_kit::base::Root::new(cx.new(|_| Spike), window, cx))
                })
                .unwrap();
            cx.activate(true);
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_millis(1500)).await;
                if let Some(path) = capture {
                    let number = handle
                        .update(cx, |_, window, _| pm_platform::window::window_number(window).unwrap())
                        .unwrap();
                    match pm_platform::window::capture(number, path.as_ref()) {
                        Ok(()) => eprintln!("captured {path}"),
                        Err(err) => eprintln!("capture failed: {err}"),
                    }
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}

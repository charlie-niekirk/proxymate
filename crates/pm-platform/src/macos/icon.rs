//! App icons as PNG through `NSWorkspace.iconForFile`.

use std::path::Path;

use objc2::AnyThread as _;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// Renders the Finder icon of `path` at `size`×`size` points, at the main
/// screen's scale, in the user's icon style (e.g. clear or tinted). NSWorkspace
/// and NSImage are safe off the main thread; this runs on a worker.
#[allow(unsafe_code)] // Two AppKit calls objc2 marks unsafe for their pointer/generic arguments.
pub fn icon_png(path: &Path, size: u32) -> Option<Vec<u8>> {
    let path = path.to_str()?;
    autoreleasepool(|_| {
        let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
        let side = f64::from(size);
        let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(side, side));
        // SAFETY: `rect` outlives the call; no context or hints are passed.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        // SAFETY: an empty properties dictionary is valid for PNG.
        let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
        Some(data.to_vec())
    })
}

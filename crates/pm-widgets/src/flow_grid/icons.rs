//! Client app icons for the Client column: `Engine::app_icon`'s PNG bytes,
//! loaded once per app on the background executor and kept as gpui images.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Image, ImageFormat, ImageSource, SharedString};

use super::FlowGrid;

/// An app's PNG icon by its name (blocking: run off the foreground thread).
pub type IconSource = Arc<dyn Fn(&str) -> Option<Arc<Vec<u8>>> + Send + Sync>;

#[derive(Default)]
pub(super) struct IconCache {
    source: Option<IconSource>,
    /// `None` while loading, or for an app without an icon.
    icons: HashMap<SharedString, Option<ImageSource>>,
}

impl IconCache {
    pub(super) fn set_source(&mut self, source: IconSource) {
        self.source = Some(source);
        self.icons.clear();
    }

    /// The app's icon, if loaded; the first ask starts loading it.
    pub(super) fn get(&mut self, name: &SharedString, cx: &mut Context<FlowGrid>) -> Option<ImageSource> {
        if let Some(icon) = self.icons.get(name) {
            return icon.clone();
        }
        let source = self.source.clone()?;
        self.icons.insert(name.clone(), None);
        let key = name.clone();
        let bytes = cx.background_spawn(async move { source(&key) });
        let name = name.clone();
        cx.spawn(async move |this, cx| {
            let Some(bytes) = bytes.await else { return };
            let image = ImageSource::from(Arc::new(Image::from_bytes(ImageFormat::Png, bytes.to_vec())));
            let _ = this.update(cx, |this, cx| {
                this.icons.icons.insert(name, Some(image));
                cx.notify();
            });
        })
        .detach();
        None
    }
}

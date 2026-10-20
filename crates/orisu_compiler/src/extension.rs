use crate::design_system::Session;
use orisu_allocator::Allocator;
use orisu_registry::utilities::Utilities;
use orisu_theme::Theme;

/// A compatibility layer: extra stylesheets, utilities and theme lookups.
pub trait Extension<'a> {
    /// Comment printed first in the output.
    fn banner(&self) -> Option<&'a str> {
        None
    }

    fn stylesheet(&self, uri: &str) -> Option<&'static str> {
        let _ = uri;
        None
    }

    /// Runs right after the built-in utilities are registered.
    fn register_utilities(&self, utilities: &mut Utilities<'a>, theme: &Theme<'a>, alloc: &'a Allocator) {
        let _ = (utilities, theme, alloc);
    }

    /// `theme(…)` with a dot path such as `colors.red.500 / 50%`.
    fn resolve_theme(&self, ds: &Session<'_, 'a>, path: &str) -> Option<&'a str> {
        let _ = (ds, path);
        None
    }
}

use crate::design_system::Session;
use orisu_allocator::Allocator;
use orisu_registry::utilities::Utilities;
use orisu_theme::Theme;

/// A compatibility layer: extra stylesheets, utilities and theme lookups.
pub trait Extension<'a> {
    fn banner(&self) -> &'a str;

    fn stylesheet(&self, uri: &str) -> Option<&'static str>;

    /// Runs right after the built-in utilities are registered.
    fn register_utilities(&self, utilities: &mut Utilities<'a>, theme: &Theme<'a>, alloc: &'a Allocator);

    /// `theme(…)` with a dot path such as `colors.red.500 / 50%`.
    fn resolve_theme(&self, ds: &Session<'_, 'a>, path: &str) -> Option<&'a str>;
}

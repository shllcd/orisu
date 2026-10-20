// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

mod legacy;
pub mod stylesheets;
mod theme_fn;

use orisu_allocator::Allocator;
use orisu_compiler::design_system::Session;
use orisu_compiler::extension::Extension;
use orisu_registry::utilities::{Registrar, Utilities};
use orisu_theme::Theme;

pub const VERSION: &str = "4.3.3";
pub const BANNER: &str = "! tailwindcss v4.3.3 | MIT License | https://tailwindcss.com ";

/// Tailwind CSS 4.3.3 on top of the orisu compiler.
pub struct Tailwind;

impl<'a> Extension<'a> for Tailwind {
    fn banner(&self) -> Option<&'a str> {
        Some(BANNER)
    }

    fn stylesheet(&self, uri: &str) -> Option<&'static str> {
        stylesheets::builtin(uri)
    }

    fn register_utilities(&self, utilities: &mut Utilities<'a>, theme: &Theme<'a>, alloc: &'a Allocator) {
        legacy::register(&mut Registrar { utilities, theme, alloc });
    }

    fn resolve_theme(&self, ds: &Session<'_, 'a>, path: &str) -> Option<&'a str> {
        theme_fn::resolve(ds, path)
    }
}

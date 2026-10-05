//! `color_struct!`: a struct of [`gpui::Hsla`] fields, each documented in one line.

/// Declares `pub struct $name { pub $field: Hsla, .. }` with the field docs given as literals
/// and a crate-private `splat` constructor (every field the same colour).
macro_rules! color_struct {
    ($(#[$meta:meta])* $name:ident { $($field:ident : $doc:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct $name {
            $(
                #[doc = $doc]
                pub $field: gpui::Hsla,
            )+
        }

        impl $name {
            /// Every field set to `color`: the blank slate an importer starts from.
            pub(crate) fn splat(color: gpui::Hsla) -> Self {
                Self { $($field: color),+ }
            }
        }
    };
}

pub(crate) use color_struct;

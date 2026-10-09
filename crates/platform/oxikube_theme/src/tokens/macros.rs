//! `color_struct!`: a struct of [`gpui::Hsla`] fields, each documented in one line.

/// Declares `pub struct $name { pub $field: Hsla, .. }` with the field docs given as literals
/// a crate-private `splat` constructor (every field the same colour) and `push_colors` (every
/// field in order, for pairing the same slots of two themes).
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

            /// Appends every field, in declaration order.
            pub(crate) fn push_colors(&self, out: &mut Vec<gpui::Hsla>) {
                $(out.push(self.$field);)+
            }
        }
    };
}

pub(crate) use color_struct;

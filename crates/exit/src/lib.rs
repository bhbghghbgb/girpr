//! Shared exit-table *mechanism*, parameterised by each caller's own values.
//!
//! An exit table has two parts: a mechanism that keeps an enum, its exhaustive
//! `ALL` list, its numeric mapping and its compile-time pin in sync, and the
//! values themselves — which codes exist, what they mean, and which helper
//! methods (`slug`, `describe`, …) a program hangs off them. The mechanism is
//! the same in every program; the values are each program's own contract. This
//! crate exports only the former as a macro. It holds no codes, no meanings and
//! no program logic, and it depends on nothing.
//!
//! A caller supplies the enum attributes (its docs and derives), its
//! visibility, the ordered `Variant = code` list, the attributes and
//! visibility for `ALL`, and the attributes for `code()`. The macro emits the
//! enum with `#[repr(u8)]` and the given discriminants, an `ALL` constant whose
//! length comes from the same list, a `code()` accessor, and a wildcard-free
//! compile-time check that every code is contiguous from zero. Adding or
//! renumbering a variant therefore fails the build at the table itself, before
//! any test can drift.

/// Define an exit-code table from one supplied list.
///
/// Emits the enum, its `ALL` constant, the `code()` mapping and the
/// compile-time pin. Every other method — slugs, descriptions, trust flags,
/// conversions and the process boundary — stays with the caller, which knows
/// its own contract. See the crate docs for the invocation shape.
#[macro_export]
macro_rules! define_exit_table {
    (
        $(#[$enum_attr:meta])*
        $enum_vis:vis enum $name:ident {
            $(
                $(#[$variant_attr:meta])*
                $variant:ident = $code:literal,
            )+
        }
        ALL $(#[$all_attr:meta])* $all_vis:vis,
        CODE $(#[$code_attr:meta])*;
    ) => {
        $(#[$enum_attr])*
        #[repr(u8)]
        $enum_vis enum $name {
            $(
                $(#[$variant_attr])*
                $variant = $code,
            )+
        }

        impl $name {
            $(#[$all_attr])*
            $all_vis const ALL: [$name; [$($name::$variant),+].len()] = [
                $($name::$variant,)+
            ];

            $(#[$code_attr])*
            pub const fn code(self) -> i32 {
                self as i32
            }
        }

        /// The exit-code table is pinned at compile time: every variant is
        /// named after its documented number (the `match` has no wildcard),
        /// and `ALL` is numbered contiguously from zero. A new or renumbered
        /// variant stops the build here, before any test can drift.
        const _: () = {
            let mut i = 0;
            while i < $name::ALL.len() {
                let documented = match $name::ALL[i] {
                    $($name::$variant => $code,)+
                };
                assert!($name::ALL[i] as u8 == documented);
                assert!(documented as usize == i);
                i += 1;
            }
        };
    };
}

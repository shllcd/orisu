macro_rules! classify {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($variant:ident = $pat:pat,)* _ => $fallback:ident $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy)]
        $vis enum $name {
            $($variant,)*
            $fallback,
        }

        impl $name {
            const TABLE: [$name; 256] = {
                let mut table = [$name::$fallback; 256];
                let mut i = 0;
                while i < 256 {
                    table[i] = match i as u8 {
                        $($pat => $name::$variant,)*
                        _ => $name::$fallback,
                    };
                    i += 1;
                }
                table
            };
        }

        impl From<u8> for $name {
            #[inline(always)]
            fn from(byte: u8) -> Self {
                Self::TABLE[byte as usize]
            }
        }

        impl From<&u8> for $name {
            #[inline(always)]
            fn from(byte: &u8) -> Self {
                Self::TABLE[*byte as usize]
            }
        }
    };
}

pub(crate) use classify;

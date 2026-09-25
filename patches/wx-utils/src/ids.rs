/// Generates sequential `i32` constants starting from a base value.
///
/// ```rust
/// use wx_utils::seq_ids;
///
/// mod ids {
///     wx_utils::seq_ids!(1000 => OPEN, CLOSE, SAVE);
///     // OPEN == 1000, CLOSE == 1001, SAVE == 1002
/// }
/// ```
#[macro_export]
macro_rules! seq_ids {
    ($base:expr => $($name:ident),+ $(,)?) => {
        $crate::seq_ids!(@step $base; $($name),+);
    };
    (@step $n:expr; $name:ident) => {
        pub const $name: i32 = $n;
    };
    (@step $n:expr; $name:ident, $($rest:ident),+) => {
        pub const $name: i32 = $n;
        $crate::seq_ids!(@step $n + 1; $($rest),+);
    };
}

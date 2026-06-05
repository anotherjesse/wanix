use std::ffi::OsString;

pub(crate) fn named_arg<T: Copy>(arg: &OsString, options: &[(&str, T)]) -> Option<T> {
    let arg = arg.to_str()?;
    options
        .iter()
        .find_map(|(name, option)| (*name == arg).then_some(*option))
}

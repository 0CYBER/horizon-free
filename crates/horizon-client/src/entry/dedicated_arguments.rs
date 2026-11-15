//! Tokenization shared by dedicated preflight and pre-registration process capture.
//! An option's operand is consumed even when it looks like another flag.
pub(crate) fn parse(args: &[String]) -> impl Iterator<Item = Result<(&str, Option<&str>), String>> {
    let mut args = args.iter();
    std::iter::from_fn(move || {
        let arg = args.next()?;
        let name = arg.trim_start_matches('-');
        Some(match name {
            "world" | "universe" | "port" | "serverId" | "pidFile" => args
                .next()
                .map(|value| (name, Some(value.as_str())))
                .ok_or_else(|| format!("{arg} needs a value")),
            _ => Ok((name, None)),
        })
    })
}

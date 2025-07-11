#![cfg(feature = "capture-io")]

prelude!();

use crate::termcolor::{ColorChoice, StandardStream};

macro_rules! capture {
    ($($tt:tt)*) => {{
        let capture = crate::modules::capture_io::CaptureIo::new();
        let module = crate::modules::capture_io::module(&capture).context("building capture module")?;

        let mut context = Context::with_config(false).context("building context")?;
        context.install(module).context("installing module")?;

        let runtime = Arc::try_new(context.runtime()?)?;

        let source = Source::memory(stringify!($($tt)*)).context("building source")?;

        let mut sources = Sources::new();
        sources.insert(source).context("inserting source")?;

        let mut diagnostics = Diagnostics::new();

        let mut options = Options::default();
        options.script(true);

        let unit = prepare(&mut sources)
            .with_context(&context)
            .with_diagnostics(&mut diagnostics)
            .with_options(&options)
            .build();

        if !diagnostics.is_empty() {
            let mut writer = StandardStream::stderr(ColorChoice::Always);
            diagnostics.emit(&mut writer, &sources)?;
        }

        let unit = unit?;
        let unit = Arc::try_new(unit)?;
        let mut vm = Vm::new(runtime, unit);

        vm.call(Hash::EMPTY, ())?;
        capture.drain_utf8()?
    }};
}

macro_rules! test_case {
    ($expected:expr, {$($prefix:tt)*}, $($format:tt)*) => {{
        let string = capture!($($prefix)* println!($($format)*));
        assert_eq!(string, concat!($expected, "\n"), "Expecting println!");

        let string = capture!($($prefix)* print!($($format)*));
        assert_eq!(string, $expected, "Expecting print!");

        let string: String = rune!($($prefix)* format!($($format)*));
        assert_eq!(string, $expected, "Expecting format!");
    }}
}

#[test]
fn format_macros() -> Result<()> {
    test_case!("Hello World!", {}, "Hello World!");
    test_case!("Hello World!", {}, "Hello {}!", "World");
    test_case!(
        "Hello World!",
        {
            let pos = "Hello";
        },
        "{pos} {}!",
        "World"
    );
    test_case!(
        "Hello World!",
        {
            let pos = "Not Hello";
        },
        "{pos} {}!",
        "World",
        pos = "Hello"
    );
    Ok(())
}

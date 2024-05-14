use std::io::Write;

use codemap::CodeMap;
use codemap_diagnostic::{ColorConfig, Diagnostic, Emitter, Level, SpanLabel, SpanStyle};
use rust_sitter::errors::{ParseError, ParseErrorReason};

#[rust_sitter::grammar("command")]
pub mod grammar {
    #[rust_sitter::language]
    pub enum CommandExpr {
        StepInto(#[rust_sitter::leaf(text = "t")] ()),
        Go(#[rust_sitter::leaf(text = "g")] ()),
        SetBreakpoint(#[rust_sitter::leaf(text = "bp")] (), Box<EvalExpr>),
        ListBreakpoints(#[rust_sitter::leaf(text = "bl")] ()),
        ClearBreakpoint(#[rust_sitter::leaf(text = "bc")] (), Box<EvalExpr>),
        DisplaySpecificRegister(#[rust_sitter::leaf(text = "r")] (), #[rust_sitter::leaf(pattern = "([a-zA-Z]+)", transform = parse_sym)] String),
        DisplayRegisters(#[rust_sitter::leaf(text = "r")] ()),
        StackWalk(#[rust_sitter::leaf(text = "k")] ()),
        DisplayBytes(#[rust_sitter::leaf(text = "db")] (), Box<EvalExpr>),
        Evaluate(#[rust_sitter::leaf(text = "?")] (), Box<EvalExpr>),
        ListNearest(#[rust_sitter::leaf(text = "ln")] (), Box<EvalExpr>),
        Unassemble(#[rust_sitter::leaf(text = "u")] (), Box<EvalExpr>),
        UnassembleContinue(#[rust_sitter::leaf(text = "u")] ()),
        ListSource(#[rust_sitter::leaf(text = "lsa")] (), Box<EvalExpr>),
        Quit(#[rust_sitter::leaf(text = "q")] ()),
    }

    #[rust_sitter::language]
    pub enum EvalExpr {
        Number(#[rust_sitter::leaf(pattern = r"(\d+|0x[0-9a-fA-F]+)", transform = parse_int)] u64),
        Symbol(#[rust_sitter::leaf(pattern = r"(([a-zA-Z0-9_@#.]+!)?[a-zA-Z0-9_@#.]+)", transform = parse_sym)] String),
        #[rust_sitter::prec_left(1)]
        Add(
            Box<EvalExpr>,
            #[rust_sitter::leaf(text = "+")] (),
            Box<EvalExpr>,
        ),
    }

    #[rust_sitter::extra]
    struct Whitespace {
        #[rust_sitter::leaf(pattern = r"\s")]
        _whitespace: (),
    }

    fn parse_int(text: &str) -> u64 {
        let text = text.trim();
        if text.starts_with("0x") {
            let text = text.split_at(2).1;
            u64::from_str_radix(text, 16).unwrap()
        } else {
            text.parse().unwrap()
        }
    }

    fn parse_sym(text: &str) -> String {
        text.to_owned()
    }
}

// This came from https://github.com/hydro-project/rust-sitter/blob/main/example/src/main.rs
fn convert_parse_error_to_diagnostics(
    file_span: &codemap::Span,
    error: &ParseError,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match &error.reason {
        ParseErrorReason::MissingToken(tok) => diagnostics.push(Diagnostic {
            level: Level::Error,
            message: format!("Missing token: \"{tok}\""),
            code: Some("S000".to_string()),
            spans: vec![SpanLabel {
                span: file_span.subspan(error.start as u64, error.end as u64),
                style: SpanStyle::Primary,
                label: Some(format!("missing \"{tok}\"")),
            }],
        }),

        ParseErrorReason::UnexpectedToken(tok) => diagnostics.push(Diagnostic {
            level: Level::Error,
            message: format!("Unexpected token: \"{tok}\""),
            code: Some("S000".to_string()),
            spans: vec![SpanLabel {
                span: file_span.subspan(error.start as u64, error.end as u64),
                style: SpanStyle::Primary,
                label: Some(format!("unexpected \"{tok}\"")),
            }],
        }),

        ParseErrorReason::FailedNode(errors) => {
            if errors.is_empty() {
                diagnostics.push(Diagnostic {
                    level: Level::Error,
                    message: "Failed to parse node".to_string(),
                    code: Some("S000".to_string()),
                    spans: vec![SpanLabel {
                        span: file_span.subspan(error.start as u64, error.end as u64),
                        style: SpanStyle::Primary,
                        label: Some("failed".to_string()),
                    }],
                })
            } else {
                for error in errors {
                    convert_parse_error_to_diagnostics(file_span, error, diagnostics);
                }
            }
        }
    }
}

pub fn read_command() -> grammar::CommandExpr {
    let stdin = std::io::stdin();
    loop {
        print!("> ");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        stdin.read_line(&mut input).unwrap();
        let input = input.trim().to_string();
        if !input.is_empty() {
            let cmd = grammar::parse(&input);
            match cmd {
                Ok(c) => return c,
                Err(errs) => {
                    // This came from https://github.com/hydro-project/rust-sitter/blob/main/example/src/main.rs
                    let mut codemap = CodeMap::new();
                    let file_span = codemap.add_file("<input>".to_string(), input.to_string());
                    let mut diagnostics = vec![];
                    for error in errs {
                        convert_parse_error_to_diagnostics(
                            &file_span.span,
                            &error,
                            &mut diagnostics,
                        );
                    }

                    let mut emitter = Emitter::stderr(ColorConfig::Always, Some(&codemap));
                    emitter.emit(&diagnostics);
                }
            }
        }
    }
}

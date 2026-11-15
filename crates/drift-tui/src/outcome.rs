use lunatui_core::{Attrs, Style};
use lunatui_theme::Token;
use lunatui_widgets::RenderCx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Applied,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outcome {
    pub text: String,
    pub tone: Tone,
}

impl Outcome {
    pub fn style(&self, cx: &RenderCx<'_>, base: Style) -> Style {
        let token = match self.tone {
            Tone::Applied => Token::Ok,
            Tone::Error => Token::Fail,
            Tone::Cancelled => Token::Warn,
        };
        base.fg(cx.token(token))
            .with_attrs(Attrs::new().set_bold(true))
    }
}

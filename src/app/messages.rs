#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Error,
    Loading,
}

pub struct AppMessage {
    pub kind: MessageKind,
    pub text: String,
}

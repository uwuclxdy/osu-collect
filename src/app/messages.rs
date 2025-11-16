#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Info,
    Error,
}

pub struct HomeMessage {
    pub kind: MessageKind,
    pub text: String,
}

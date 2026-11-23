use std::fmt;
use std::fmt::Formatter;

#[derive(PartialEq, Debug, Clone)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

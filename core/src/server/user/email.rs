use std::fmt;

/// Representation of an email
/// This helps ensure the email at least has an '@' in it...
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Email {
    pub local_part: String,
    pub domain_part: String,
}

impl fmt::Display for Email {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl Email {
    pub fn new(local_part: String, domain_part: String) -> Email {
        Email {
            local_part: local_part,
            domain_part: domain_part,
        }
    }

    fn to_string(&self) -> String {
        let mut res = self.local_part.clone();
        res.push('@');
        res.push_str(&self.domain_part[..]);
        res
    }
}

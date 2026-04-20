use crate::auth::session;

pub fn verify() {
    let _ = session::current();
}

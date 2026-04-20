use crate::auth::jwt;

pub fn handle() {
    jwt::verify();
}

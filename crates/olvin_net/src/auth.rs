//! Who may join. The server asks an [`Authenticator`] with the token the
//! client sent in its `Hello`. These types live on the server side only: a
//! client build never constructs them, so no server secret ends up in it (N4).

use crate::crypto::ct_eq;

/// What the server knows about a client that wants in.
#[derive(Debug)]
pub struct AuthRequest<'a> {
    /// The opaque token from `NetConfig::token` on the client (a password, a
    /// ticket from your account service...). Empty if the client sent none.
    pub token: &'a [u8],
    /// The name the client asked for. Untrusted display text, already length-limited.
    pub name: &'a str,
    /// The peer address, if the transport has one.
    pub addr: Option<&'a str>,
}

/// Who the server decided the client is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthInfo {
    /// The display name to use (an account name instead of the requested one).
    pub name: String,
}

pub type AuthResult = Result<AuthInfo, String>;

/// Decides whether a client may join. `Err(reason)` refuses and the reason is
/// shown to the player, so keep it free of internals.
pub trait Authenticator: 'static {
    fn authenticate(&mut self, request: &AuthRequest) -> AuthResult;
}

/// Accepts everybody under the name they asked for. Fine for a LAN game.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAll;

impl Authenticator for AllowAll {
    fn authenticate(&mut self, request: &AuthRequest) -> AuthResult {
        Ok(AuthInfo {
            name: request.name.to_owned(),
        })
    }
}

/// A shared password: the client's token must equal it.
#[derive(Clone)]
pub struct SharedPassword {
    password: Vec<u8>,
}

impl SharedPassword {
    pub fn new(password: impl Into<Vec<u8>>) -> Self {
        Self {
            password: password.into(),
        }
    }
}

impl Authenticator for SharedPassword {
    fn authenticate(&mut self, request: &AuthRequest) -> AuthResult {
        if ct_eq(request.token, &self.password) {
            Ok(AuthInfo {
                name: request.name.to_owned(),
            })
        } else {
            Err("wrong password".into())
        }
    }
}

/// Any closure is an authenticator: `net.set_authenticator(|r: &AuthRequest| ...)`.
impl<F> Authenticator for F
where
    F: FnMut(&AuthRequest) -> AuthResult + 'static,
{
    fn authenticate(&mut self, request: &AuthRequest) -> AuthResult {
        self(request)
    }
}

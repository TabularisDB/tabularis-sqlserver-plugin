//! NTLM with explicit `DOMAIN\user` credentials, on every platform.
//!
//! mssql-tds' built-in integrated auth is Kerberos-only on Linux/macOS
//! (GSSAPI). This plugs Devolutions' pure-Rust `sspi` NTLM client into its
//! `SecurityContextProvider` hook instead, so no system library is needed.
//! Channel bindings (Extended Protection) are not sent.

use mssql_tds::security::{
    IntegratedAuthConfig, SecurityContext, SecurityContextProvider, SecurityError, SspiAuthToken,
};
use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, ClientRequestFlags, CredentialUse,
    DataRepresentation, Ntlm, SecurityBuffer, SecurityStatus, Sspi, SspiImpl, Username,
};

pub struct NtlmProvider {
    identity: AuthIdentity,
}

impl NtlmProvider {
    /// `username` is `DOMAIN\user` or `user@domain`.
    pub fn new(username: &str, password: &str) -> Result<Self, String> {
        let username = Username::parse(username)
            .map_err(|error| format!("invalid NTLM username {username:?}: {error}"))?;
        Ok(Self {
            identity: AuthIdentity {
                username,
                password: password.to_string().into(),
            },
        })
    }
}

impl SecurityContextProvider for NtlmProvider {
    fn create_context(
        &self,
        _config: &IntegratedAuthConfig,
        server: &str,
        port: u16,
    ) -> Result<Box<dyn SecurityContext>, SecurityError> {
        let mut ntlm = Ntlm::new();
        let credentials = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&self.identity)
            .execute(&mut ntlm)
            .map_err(|error| SecurityError::AcquireCredentialsFailed {
                code: error.error_type as u32,
                message: error.to_string(),
            })?
            .credentials_handle;
        Ok(Box::new(NtlmContext {
            ntlm,
            credentials,
            spn: format!("MSSQLSvc/{server}:{port}"),
            complete: false,
        }))
    }
}

struct NtlmContext {
    ntlm: Ntlm,
    credentials: Option<AuthIdentityBuffers>,
    spn: String,
    complete: bool,
}

impl SecurityContext for NtlmContext {
    fn package_name(&self) -> &str {
        "NTLM"
    }

    fn generate_token(
        &mut self,
        server_token: Option<&[u8]>,
    ) -> Result<SspiAuthToken, SecurityError> {
        let failed = |error: sspi::Error| SecurityError::InitContextFailed {
            code: error.error_type as u32,
            message: error.to_string(),
        };
        let mut input = vec![SecurityBuffer::new(
            server_token.unwrap_or_default().to_vec(),
            BufferType::Token,
        )];
        let mut output = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
        let mut builder = self
            .ntlm
            .initialize_security_context()
            .with_credentials_handle(&mut self.credentials)
            .with_context_requirements(ClientRequestFlags::ALLOCATE_MEMORY)
            .with_target_data_representation(DataRepresentation::Native)
            .with_target_name(&self.spn)
            .with_output(&mut output);
        if server_token.is_some() {
            builder = builder.with_input(&mut input);
        }
        let status = self
            .ntlm
            .initialize_security_context_impl(&mut builder)
            .map_err(failed)?
            .resolve_to_result()
            .map_err(failed)?
            .status;
        if matches!(
            status,
            SecurityStatus::CompleteAndContinue | SecurityStatus::CompleteNeeded
        ) {
            self.ntlm.complete_auth_token(&mut output).map_err(failed)?;
        }
        self.complete = !matches!(
            status,
            SecurityStatus::ContinueNeeded | SecurityStatus::CompleteAndContinue
        );
        let data = output.pop().map(|buffer| buffer.buffer).unwrap_or_default();
        Ok(SspiAuthToken::new(data, self.complete))
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn spn(&self) -> &str {
        &self.spn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::random_secret;
    use sspi::ServerRequestFlags;

    /// Runs the client against sspi's NTLM server, as SQL Server would.
    fn handshake(user: &str, client_password: &str, server_password: &str) -> sspi::Result<()> {
        let client = NtlmProvider::new(user, client_password).expect("username parses");
        let mut client = client
            .create_context(&IntegratedAuthConfig::new(), "db.example.com", 1433)
            .expect("context");

        let mut server = Ntlm::new();
        let server_identity = AuthIdentity {
            username: Username::parse(user).expect("username parses"),
            password: server_password.to_string().into(),
        };
        let mut server_credentials = server
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Inbound)
            .with_auth_data(&server_identity)
            .execute(&mut server)?
            .credentials_handle;

        let mut token = client.generate_token(None).expect("negotiate");
        assert!(!token.is_complete);
        loop {
            let mut input = vec![SecurityBuffer::new(token.data, BufferType::Token)];
            let mut output = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
            let builder = server
                .accept_security_context()
                .with_credentials_handle(&mut server_credentials)
                .with_context_requirements(ServerRequestFlags::ALLOCATE_MEMORY)
                .with_target_data_representation(DataRepresentation::Native)
                .with_input(&mut input)
                .with_output(&mut output);
            let status = server
                .accept_security_context_impl(builder)?
                .resolve_to_result()?
                .status;
            if matches!(
                status,
                SecurityStatus::CompleteAndContinue | SecurityStatus::CompleteNeeded
            ) {
                server.complete_auth_token(&mut output)?;
            }
            if status != SecurityStatus::ContinueNeeded {
                assert!(client.is_complete());
                return Ok(());
            }
            token = client
                .generate_token(Some(&output[0].buffer))
                .expect("authenticate");
        }
    }

    #[test]
    fn completes_a_handshake_with_matching_credentials() {
        let secret = random_secret();
        handshake(r"CONTOSO\alice", &secret, &secret).expect("server accepts");
    }

    #[test]
    fn a_wrong_password_is_rejected() {
        let (client_secret, server_secret) = (random_secret(), random_secret());
        assert_ne!(client_secret, server_secret);
        assert!(handshake(r"CONTOSO\alice", &client_secret, &server_secret).is_err());
    }

    #[test]
    fn rejects_an_unparseable_username() {
        assert!(NtlmProvider::new(r"CONTOSO\alice\bob", "x").is_err());
    }
}

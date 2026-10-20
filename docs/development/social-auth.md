# GitHub and Google sign-in

Genie supports GitHub and Google authorization-code sign-in with PKCE S256. The backend exchanges the code and fetches the authenticated profile from fixed provider APIs, then issues its normal session cookie. It does not expose or persist provider access/refresh tokens, consume ID-token claims, or implement a generic OIDC client. See [the feasibility research](social-auth-research.md) for official sources and requirements for Microsoft, Apple and OIDC extensions.

## Configure a deployment

Create the first administrator through the existing local setup. Social registration cannot bootstrap an administrator or an installation with no active users.

Set the deployment's `<data>/config.json` (merge these fields with existing configuration):

```json
{
  "publicUrl": "https://genie.example.com",
  "allowHosts": ["genie.example.com"],
  "socialAuth": {
    "allowRegistration": false,
    "github": {
      "clientId": "YOUR_GITHUB_CLIENT_ID",
      "clientSecretEnv": "GENIE_GITHUB_CLIENT_SECRET"
    },
    "google": {
      "clientId": "YOUR_GOOGLE_CLIENT_ID",
      "clientSecretEnv": "GENIE_GOOGLE_CLIENT_SECRET"
    }
  }
}
```

Supply the two secret values as environment variables of the **server process**, using your deployment's secret manager or private environment file. Never commit secrets. `clientSecretEnv` names a variable, not the secret itself. Omit either provider's object to disable it. An incomplete configuration hides the provider's button. Configuration and environment are read when the server starts; restart after changing them.

`publicUrl` must be an HTTPS origin, with no path prefix, credentials, query or fragment. HTTP is permitted only for `localhost`, `127.0.0.1` or `[::1]` development origins. Callback construction ignores incoming Host and forwarded headers. Configure the existing Host allowlist and HTTPS reverse proxy for the deployment. Secure cookies are enabled when `publicUrl` uses HTTPS.

### GitHub

Create an OAuth App in GitHub's developer settings, with the website URL above and this authorization callback URL:

```text
https://genie.example.com/api/auth/social/github/callback
```

Copy its client ID into configuration and provide the client secret through the named environment variable. Genie requests `read:user`; it does not need repository permissions or private email access. This is account authentication, independent of repository integrations and GitHub Copilot model credentials.

### Google

Create a Google Cloud OAuth web client, configure its consent screen/audience, and register this exact authorized redirect URI:

```text
https://genie.example.com/api/auth/social/google/callback
```

Copy its client ID and provide the secret through the named environment variable. Genie requests only `openid profile`, without offline access. Configure test users while the app is in testing, then complete the applicable publishing/branding requirements before production use. A valid local flow does not prove the production consent-screen configuration.

For localhost, substitute your exact development origin and port in both Genie and each provider's registered callback.

## Accounts and registration

- Existing users sign in with their password, open **Profile settings → Sign-in and password**, and explicitly connect GitHub or Google. The browser session must remain active until the callback completes. After that, the connected provider can sign in to the same account.
- `allowRegistration: false` is the default. Unknown identities cannot create accounts. Password/invitation login continues to work.
- Set `allowRegistration: true` only when public account creation is intended. The first sign-in creates a normal user with a generated unique login, the provider's display name and no password, email, server admin privileges or project memberships. An administrator must grant project access. Users can rename their login through the existing profile controls.
- Identity uses GitHub's numeric user ID or Google's stable `sub`. Matching email addresses or display names never merge accounts. An identity already linked elsewhere cannot be moved by signing in as another user.
- Disabled accounts cannot sign in or link providers. Deleting an account removes its identity mappings. Renaming a provider profile does not change the mapping.
- There is no self-service unlink operation in this version. Passwordless users need administrator assistance to establish a password or recover access if their provider becomes unavailable. Provider credentials do not grant project access or repository permissions.

## Verification and operation

Run the Rust auth/domain/API tests and web checks from the repository CI workflow. Tests use temporary databases and a local mock provider; no production credentials are needed. The schema upgrade from server version 6 to 7 adds identity mappings and preserves users, sessions and memberships. Older binaries reject schema 7; take a compatible backup before deploying and restore it for a binary rollback.

Each authorization attempt lasts ten minutes, is bound to its browser and provider, and can be consumed once. Attempts are held in bounded process memory; restarting the server requires starting login again. One browser flow cookie means a newer attempt supersedes an older tab. Multiple independent backend replicas would require shared attempt storage or sticky routing.

After configuring real credentials, verify both providers in a browser: connect an existing account, log out and sign in again; test cancellation, denied registration and (if enabled) creation of a user with no project access. A failure returns a generic sign-in error to avoid exposing provider responses or tokens. Check provider credentials, exact redirect URI, consent audience, browser cookie settings and server reachability when diagnosing it. This branch does not activate or claim a live-provider test without deployment credentials.

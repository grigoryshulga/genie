# Social authentication feasibility

Researched against first-party documentation on 2026-10-18. This note separates protocol feasibility from deployment activation; it does not claim a successful login against live provider credentials.

## Recommendation

Implement GitHub and Google first, using server-side authorization-code exchanges and the existing Genie session mechanism. Keep each provider disabled until its deployment has complete credentials and a registered callback URL. This is an implementation recommendation based on the protocols below: a hosted identity broker is not required. Provider registration remains an operator action; code alone cannot activate a deployment's OAuth application.

Microsoft and a configured OpenID Connect issuer are reasonable extensions. Apple has additional account, key and domain prerequisites and should be a separate integration rather than an assumed drop-in OIDC provider.

## Provider requirements

### GitHub

Register an OAuth application under a GitHub account or organization, supplying the website and authorization callback URL. GitHub currently documents up to ten callback URLs. Use separate applications for deployment environments where practical. [Creating an OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app)

The web flow redirects to GitHub, returns a temporary code, and exchanges it server-side using the application's client ID and client secret. Send unpredictable `state` and PKCE with `S256`, then check state and redeem with the original `code_verifier`. Re-fetch the authenticated user on every login. Explicitly register the exact callback and disable wildcard matching. GitHub now supports PKCE; older guidance saying it does not is obsolete. [Authorizing OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps), [PKCE announcement](https://github.blog/changelog/2025-07-14-pkce-support-for-oauth-and-github-app-authentication/)

Use the authenticated user's numeric `id` as the external identity, rather than a mutable login name. The profile API supplies the ID. If registration needs a verified email, request `user:email` and use the email API's `verified` and `primary` fields; do not assume the public profile email is present or verified. [Authenticated user API](https://docs.github.com/en/rest/users/users#get-the-authenticated-user), [Email API](https://docs.github.com/en/rest/users/emails#list-email-addresses-for-the-authenticated-user)

### Google

Create a Google Cloud project, configure its OAuth consent screen, and create a web client with a client ID, client secret and authorized redirect URI. Redirect URIs must match the registered value. Development can use localhost; production should use HTTPS and a separate project/client configuration. [Web server setup and URI rules](https://developers.google.com/identity/protocols/oauth2/web-server), [Production policy](https://developers.google.com/identity/protocols/oauth2/production-readiness/policy-compliance)

Google supports OIDC authorization-code login with `openid email profile`. Use state, PKCE `S256`, and a one-use nonce. Validate the ID token's signature, issuer, audience, expiry and nonce. Store the stable `sub` as identity; email may change, even when `email_verified` is true. Discovery publishes endpoints and signing keys. [Google OpenID Connect](https://developers.google.com/identity/openid-connect/openid-connect)

An alternative for this implementation is a server-only code exchange followed by an authenticated HTTPS request to Google's fixed `https://openidconnect.googleapis.com/v1/userinfo` endpoint. Google documents this API and its required stable `sub`. If the application discards the ID token entirely, it must not decode or trust any of its claims, and a nonce has no ID-token consumer to validate it. Trust then rests on the browser-bound state, PKCE, confidential code exchange and Google's authenticated profile response; never accept a browser-supplied access token in this flow. This is an OAuth code flow with profile lookup, not a claim of full OIDC client conformance: OIDC's documented code flow validates the ID token, and OIDC UserInfo processing compares its subject with that token. [Google UserInfo reference](https://developers.google.com/identity/openid-connect/reference), [OIDC code flow and UserInfo validation](https://openid.net/specs/openid-connect-core-1_0.html#CodeFlowSteps)

Production readiness includes branding/domain configuration; displaying the application's name or logo can require brand verification. Sensitive/restricted API scopes add separate verification requirements. Keep login scopes minimal and check the project's consent-screen status before launch. Development/test apps are exempt from verification, but this does not establish production readiness. [Brand verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/brand-verification), [Scope verification](https://support.google.com/cloud/answer/13463073), [Verification exemptions](https://support.google.com/cloud/answer/13464323)

### Microsoft

Register an application in Microsoft Entra, choose supported account types (organizational, personal or both), and add the web redirect URI. A server-side confidential client also needs a credential. Microsoft documents tenant/account prerequisites and app-registration privileges; tenant consent policies can impose additional activation gates. [App registration](https://learn.microsoft.com/en-us/entra/identity-platform/quickstart-register-app), [Authorization-code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)

Microsoft exposes OIDC discovery, authorization, token, UserInfo and signing-key endpoints. Use authorization code with PKCE; validate tokens and nonce. Multitenant `common` login needs issuer handling appropriate to the returned tenant rather than treating `common` as a fixed token issuer. Registered redirect rules apply. [Microsoft OIDC](https://learn.microsoft.com/en-us/entra/identity-platform/v2-protocols-oidc), [Redirect URI restrictions](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url)

### Apple

Web login needs a Services ID associated with a primary Apple-platform App ID enabled for Sign in with Apple, registered domains and return URLs, and a private signing key. The redirect host cannot be localhost or an IP address, so a local-only deployment needs a real domain for testing. Apple also returns authorization results using a form POST, which needs explicit callback and cookie handling. [Web account configuration](https://developer.apple.com/help/account/capabilities/configure-sign-in-with-apple-for-the-web), [Environment and keys](https://developer.apple.com/documentation/signinwithapple/configuring-your-environment-for-sign-in-with-apple), [Webpage and callback requirements](https://developer.apple.com/documentation/signinwithapple/configuring-your-webpage-for-sign-in-with-apple)

Apple lists Developer Program membership at USD 99 annually (local pricing may differ). This is an explicit cost/setup consideration absent from the GitHub/Google protocol integration itself; this research does not assert pricing guarantees for other providers. [Apple Developer Program](https://developer.apple.com/programs/)

### Generic OpenID Connect

A configured OIDC issuer can reuse discovery, code exchange, ID-token validation and UserInfo handling. It still requires client registration/credentials and provider-specific redirect and consent configuration. Identify accounts by issuer plus subject, because `sub` is only unique within its issuer. Verify UserInfo's subject matches the ID token. Do not accept an arbitrary issuer URL from an unauthenticated request. [OIDC Core](https://openid.net/specs/openid-connect-core-1_0.html), [OIDC Discovery](https://openid.net/specs/openid-connect-discovery-1_0.html)

## Implementation invariants

- Bind each short-lived, single-use authorization attempt to the initiating browser and provider; reject missing, expired, mismatched or replayed state. Preserve the PKCE verifier server-side or in protected browser state.
- Exchange codes on the backend, keep client secrets and tokens out of browser responses/logs, and create the normal application session only after identity validation. Avoid refresh tokens when only login is needed.
- Treat registration, login and linking as separate application decisions. Recommendation: never silently merge existing local accounts by a matching email; require proof of the existing account before linking. A provider identity must have a unique persistent mapping.
- Derive callback URLs from trusted operator configuration, not request-controlled host/forwarded headers; allow only local application return paths.
- Keep provider errors actionable while redacting secrets. Test cancellation, bad state, replay, provider failure, identity collision and registration policy without live external accounts.

These are application design recommendations informed by the OAuth security BCP's requirements for redirect matching, CSRF protection, PKCE and authorization-server identification. [OAuth 2.0 Security Best Current Practice, RFC 9700](https://datatracker.ietf.org/doc/html/rfc9700)

## Activation checklist

The operator must supply a deployment's public base URL and per-provider client credentials, register the resulting callback URLs, configure consent/branding, and perform a real browser login before considering that provider activated. Offline tests can establish implementation behavior but cannot prove provider account eligibility, application approval or production redirect configuration.

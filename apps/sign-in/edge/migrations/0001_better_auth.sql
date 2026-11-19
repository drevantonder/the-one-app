-- Better Auth's tables, as `bun generate.ts` wrote them, made only where absent: a network's first had them made by the
-- Worker as it started, before migrations.
create table if not exists "user" ("id" text not null primary key, "name" text not null, "email" text not null unique, "emailVerified" integer not null, "image" text, "createdAt" date not null, "updatedAt" date not null, "called" text);

create table if not exists "session" ("id" text not null primary key, "expiresAt" date not null, "token" text not null unique, "createdAt" date not null, "updatedAt" date not null, "ipAddress" text, "userAgent" text, "userId" text not null references "user" ("id") on delete cascade, "passkey" text);

create table if not exists "account" ("id" text not null primary key, "accountId" text not null, "providerId" text not null, "userId" text not null references "user" ("id") on delete cascade, "accessToken" text, "refreshToken" text, "idToken" text, "accessTokenExpiresAt" date, "refreshTokenExpiresAt" date, "scope" text, "password" text, "createdAt" date not null, "updatedAt" date not null);

create table if not exists "verification" ("id" text not null primary key, "identifier" text not null, "value" text not null, "expiresAt" date not null, "createdAt" date not null, "updatedAt" date not null);

create table if not exists "jwks" ("id" text not null primary key, "publicKey" text not null, "privateKey" text not null, "createdAt" date not null, "expiresAt" date, "alg" text, "crv" text);

create table if not exists "passkey" ("id" text not null primary key, "name" text, "publicKey" text not null, "userId" text not null references "user" ("id") on delete cascade, "credentialID" text not null, "counter" integer not null, "deviceType" text not null, "backedUp" integer not null, "transports" text, "createdAt" date, "aaguid" text, "prfKey" text, "encryptedKey" text);

create table if not exists "oauthClient" ("id" text not null primary key, "clientId" text not null unique, "clientSecret" text, "clientDiscoveryId" text, "disabled" integer, "skipConsent" integer, "enableEndSession" integer, "subjectType" text, "scopes" text, "clientCredentialsScopes" text, "userId" text references "user" ("id") on delete cascade, "createdAt" date, "updatedAt" date, "name" text, "uri" text, "icon" text, "contacts" text, "tos" text, "policy" text, "softwareId" text, "softwareVersion" text, "softwareStatement" text, "redirectUris" text not null, "postLogoutRedirectUris" text, "backchannelLogoutUri" text, "backchannelLogoutSessionRequired" integer, "tokenEndpointAuthMethod" text, "applicationType" text, "jwks" text, "jwksUri" text, "grantTypes" text, "responseTypes" text, "requirePKCE" integer, "dpopBoundAccessTokens" integer, "referenceId" text, "metadata" text);

create table if not exists "oauthResource" ("id" text not null primary key, "identifier" text not null unique, "name" text not null, "accessTokenTtl" integer, "refreshTokenTtl" integer, "signingAlgorithm" text, "signingKeyId" text, "allowedScopes" text, "customClaims" text, "dpopBoundAccessTokensRequired" integer, "disabled" integer, "createdAt" date, "updatedAt" date, "policyVersion" integer, "metadata" text);

create table if not exists "oauthClientResource" ("id" text not null primary key, "clientId" text not null references "oauthClient" ("clientId") on delete cascade, "resourceId" text not null references "oauthResource" ("identifier") on delete cascade, "metadata" text, "createdAt" date);

create table if not exists "oauthRefreshToken" ("id" text not null primary key, "token" text not null unique, "clientId" text not null references "oauthClient" ("clientId") on delete cascade, "sessionId" text references "session" ("id") on delete set null, "userId" text not null references "user" ("id") on delete cascade, "referenceId" text, "authorizationCodeId" text, "resources" text, "requestedUserInfoClaims" text, "expiresAt" date not null, "createdAt" date not null, "revoked" date, "rotatedAt" date, "rotationReplayResponse" text, "rotationReplayExpiresAt" date, "authTime" date, "confirmation" text, "scopes" text not null);

create table if not exists "oauthAccessToken" ("id" text not null primary key, "token" text not null unique, "clientId" text not null references "oauthClient" ("clientId") on delete cascade, "sessionId" text references "session" ("id") on delete set null, "userId" text references "user" ("id") on delete cascade, "referenceId" text, "authorizationCodeId" text, "resources" text, "requestedUserInfoClaims" text, "refreshId" text references "oauthRefreshToken" ("id") on delete cascade, "expiresAt" date not null, "createdAt" date not null, "revoked" date, "confirmation" text, "scopes" text not null);

create table if not exists "oauthConsent" ("id" text not null primary key, "clientId" text not null references "oauthClient" ("clientId") on delete cascade, "userId" text references "user" ("id") on delete cascade, "referenceId" text, "resources" text, "requestedUserInfoClaims" text, "scopes" text not null, "createdAt" date not null, "updatedAt" date not null);

create table if not exists "oauthClientAssertion" ("id" text not null primary key, "expiresAt" date not null);

create table if not exists "deviceCode" ("id" text not null primary key, "deviceCode" text not null, "userCode" text not null, "userId" text, "expiresAt" date not null, "status" text not null, "lastPolledAt" date, "pollingInterval" integer, "clientId" text, "scope" text, "resources" text, "oauthClientId" text, "name" text, "person" text references "user" ("id") on delete cascade, "number" integer, "named" date, "place" text);

create table if not exists "device" ("id" text not null primary key, "userId" text not null references "user" ("id") on delete cascade, "kind" text not null, "name" text not null, "createdAt" date not null);

create table if not exists "invite" ("id" text not null primary key, "userId" text not null references "user" ("id") on delete cascade, "expiresAt" date not null);

create table if not exists "rateLimit" ("id" text not null primary key, "key" text not null unique, "count" integer not null, "lastRequest" bigint not null);

create index if not exists "session_userId_idx" on "session" ("userId");

create index if not exists "account_userId_idx" on "account" ("userId");

create index if not exists "verification_identifier_idx" on "verification" ("identifier");

create index if not exists "passkey_userId_idx" on "passkey" ("userId");

create index if not exists "passkey_credentialID_idx" on "passkey" ("credentialID");

create index if not exists "oauthClient_userId_idx" on "oauthClient" ("userId");

create index if not exists "oauthClientResource_clientId_idx" on "oauthClientResource" ("clientId");

create index if not exists "oauthClientResource_resourceId_idx" on "oauthClientResource" ("resourceId");

create index if not exists "oauthRefreshToken_clientId_idx" on "oauthRefreshToken" ("clientId");

create index if not exists "oauthRefreshToken_sessionId_idx" on "oauthRefreshToken" ("sessionId");

create index if not exists "oauthRefreshToken_userId_idx" on "oauthRefreshToken" ("userId");

create index if not exists "oauthRefreshToken_authorizationCodeId_idx" on "oauthRefreshToken" ("authorizationCodeId");

create index if not exists "oauthAccessToken_clientId_idx" on "oauthAccessToken" ("clientId");

create index if not exists "oauthAccessToken_sessionId_idx" on "oauthAccessToken" ("sessionId");

create index if not exists "oauthAccessToken_userId_idx" on "oauthAccessToken" ("userId");

create index if not exists "oauthAccessToken_authorizationCodeId_idx" on "oauthAccessToken" ("authorizationCodeId");

create index if not exists "oauthAccessToken_refreshId_idx" on "oauthAccessToken" ("refreshId");

create index if not exists "oauthConsent_clientId_idx" on "oauthConsent" ("clientId");

create index if not exists "oauthConsent_userId_idx" on "oauthConsent" ("userId");

create unique index if not exists "oauthClientResource_clientId_resourceId_uidx" on "oauthClientResource" ("clientId", "resourceId");

create unique index if not exists "deviceCode_deviceCode_uidx" on "deviceCode" ("deviceCode");

create unique index if not exists "deviceCode_userCode_uidx" on "deviceCode" ("userCode");

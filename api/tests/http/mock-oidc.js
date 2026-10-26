// A stand-in for Google's OpenID Connect endpoints, so sign-in can be tested end to end without
// network access. Tests choose who signs in by adding sub/email/email_verified to the authorize
// URL, and can tamper with the ID token (nonce, aud, iss) to check that the API refuses it.
import { createHash, generateKeyPairSync, randomBytes, sign } from 'node:crypto';
import { createServer } from 'node:http';

const port = Number(process.env.MOCK_OIDC_PORT || 4101);
const issuer = `http://127.0.0.1:${port}`;
const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
const jwk = { ...publicKey.export({ format: 'jwk' }), kid: 'test-key', alg: 'RS256', use: 'sig' };
const codes = new Map();
const b64 = (b) => Buffer.from(b).toString('base64url');

function idToken(claims) {
  const header = b64(JSON.stringify({ alg: 'RS256', kid: 'test-key', typ: 'JWT' }));
  const body = b64(JSON.stringify(claims));
  return `${header}.${body}.${b64(sign('sha256', Buffer.from(`${header}.${body}`), privateKey))}`;
}

const json = (res, status, body) => {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(body));
};

createServer((req, res) => {
  const url = new URL(req.url, issuer);
  if (url.pathname === '/.well-known/openid-configuration') {
    return json(res, 200, {
      issuer,
      authorization_endpoint: `${issuer}/authorize`,
      token_endpoint: `${issuer}/token`,
      jwks_uri: `${issuer}/jwks`,
      response_types_supported: ['code'],
      subject_types_supported: ['public'],
      id_token_signing_alg_values_supported: ['RS256'],
      code_challenge_methods_supported: ['S256'],
    });
  }
  if (url.pathname === '/jwks') return json(res, 200, { keys: [jwk] });
  if (url.pathname === '/authorize') {
    const p = Object.fromEntries(url.searchParams);
    const code = b64(randomBytes(16));
    codes.set(code, p);
    const back = new URL(p.redirect_uri);
    back.searchParams.set('code', code);
    back.searchParams.set('state', p.state);
    res.writeHead(302, { location: back.toString() });
    return res.end();
  }
  if (url.pathname === '/token' && req.method === 'POST') {
    let raw = '';
    req.on('data', (c) => { raw += c; });
    req.on('end', () => {
      const form = new URLSearchParams(raw);
      const p = codes.get(form.get('code'));
      codes.delete(form.get('code'));
      const basic = Buffer.from((req.headers.authorization || '').replace(/^Basic /, ''), 'base64').toString();
      const clientId = decodeURIComponent(basic.split(':')[0] || '') || form.get('client_id');
      if (!p || clientId !== p.client_id) return json(res, 400, { error: 'invalid_grant' });
      const challenge = b64(createHash('sha256').update(form.get('code_verifier') || '').digest());
      if (challenge !== p.code_challenge) return json(res, 400, { error: 'invalid_grant', error_description: 'PKCE' });
      const now = Math.floor(Date.now() / 1000);
      return json(res, 200, {
        access_token: b64(randomBytes(16)),
        token_type: 'Bearer',
        expires_in: 3600,
        id_token: idToken({
          iss: p.iss || issuer,
          aud: p.aud || p.client_id,
          sub: p.sub,
          email: p.email,
          email_verified: p.email_verified !== 'false',
          name: p.name || 'Google Person',
          nonce: p.bad_nonce ? 'not-the-nonce' : p.nonce,
          iat: now,
          exp: now + 600,
        }),
      });
    });
    return undefined;
  }
  res.writeHead(404);
  return res.end();
}).listen(port, '127.0.0.1', () => console.log(`mock oidc on ${issuer}`));

const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { createHash } = require('node:crypto');
const { TextEncoder } = require('node:util');
const vm = require('node:vm');
const html = readFileSync(require('node:path').join(__dirname, '../index.html'), 'utf8');
const sha = html.slice(html.indexOf('const sha256hex ='), html.indexOf('// ---------------------------------------------------------------- helpers'));
const control = html.slice(html.indexOf('function onControl('), html.indexOf('function onHostEvent('));
const hash = s => createHash('sha256').update(s).digest('hex');
function challenge(code, required = true) {
  const elements = { secondFactor: { value: code, focus() {} }, msg: {}, go: {} };
  const messages = [];
  let closed = false;
  vm.runInNewContext(sha + control + ';onControl(challenge, password);', {
    TextEncoder, console,
    $: id => elements[id],
    send: message => messages.push(message),
    ws: { close() { closed = true; } },
    challenge: { t: 'challenge', salt: 'salt', nonce: 'fresh-nonce', two_factor: required },
    password: 'password',
  });
  return { messages: JSON.parse(JSON.stringify(messages)), closed };
}
test('viewer binds authenticator code to password and challenge nonce', () => {
  const actual = challenge('287 082');
  const secret = hash('salt:password');
  assert.equal(actual.messages[0].proof, hash(`${secret}:fresh-nonce`));
  assert.equal(actual.messages[0].second_factor_proof,
    hash(`${secret}:fresh-nonce:second_factor:${hash('287082')}`));
  assert.equal(actual.closed, false);
});
test('viewer normalizes recovery codes without sending the plaintext code', () => {
  const code = '01234567-89ABCDEF-01234567-89ABCDEF';
  const actual = challenge(code);
  assert.equal(actual.messages[0].second_factor_proof,
    hash(`${hash('salt:password')}:fresh-nonce:second_factor:${hash(code.replaceAll('-', '').toLowerCase())}`));
  assert.equal(JSON.stringify(actual.messages).includes(code), false);
});
test('missing or malformed second factors never send an authentication message', () => {
  for (const code of ['', '12345', 'not-a-code']) {
    const actual = challenge(code);
    assert.equal(actual.messages.length, 0);
    assert.equal(actual.closed, true);
  }
});
test('password-only hosts remain compatible', () => {
  const actual = challenge('', false);
  assert.equal(actual.messages.length, 1);
  assert.equal(actual.messages[0].second_factor_proof, null);
});

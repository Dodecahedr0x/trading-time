// Wallet Standard mock: registered in the page, signing and sending in Node with the e2e keypair.
import { readFileSync } from 'node:fs';
import nacl from 'tweetnacl';
import bs58 from 'bs58';

const envPath = new URL('../target/dev-stack/env.json', import.meta.url);

export function loadEnv() {
  return JSON.parse(readFileSync(envPath, 'utf8'));
}

async function rpc(url, method, params) {
  const r = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(`${method}: ${JSON.stringify(j.error)}`);
  return j.result;
}

/** Legacy/v0 wire tx: compact-u16 signature count, signatures, message. Signs slot 0 (fee payer). */
export function signWire(bytes, secretKey) {
  const tx = Uint8Array.from(bytes);
  let n = 0, shift = 0, i = 0;
  for (;;) {
    const b = tx[i++];
    n |= (b & 0x7f) << shift;
    if (!(b & 0x80)) break;
    shift += 7;
  }
  if (n < 1) throw new Error('transaction has no signature slots');
  const message = tx.subarray(i + 64 * n);
  tx.set(nacl.sign.detached(message, secretKey), i);
  return { tx, signature: tx.slice(i, i + 64) };
}

export async function installWallet(page, env) {
  const secret = Uint8Array.from(JSON.parse(readFileSync(env.e2e_keypair, 'utf8')));
  const kp = nacl.sign.keyPair.fromSecretKey(secret);
  const address = bs58.encode(kp.publicKey);

  await page.exposeFunction('__ttSign', (bytes) => Array.from(signWire(bytes, kp.secretKey).tx));
  await page.exposeFunction('__ttSignAndSend', async (bytes) => {
    const { tx, signature } = signWire(bytes, kp.secretKey);
    await rpc(env.rpc_url, 'sendTransaction', [Buffer.from(tx).toString('base64'), { encoding: 'base64', preflightCommitment: 'confirmed' }]);
    return Array.from(signature);
  });

  await page.addInitScript(({ address, publicKey }) => {
    const version = '1.0.0';
    const chains = ['solana:localnet'];
    const listeners = {};
    const account = { address, publicKey: new Uint8Array(publicKey), chains, features: ['solana:signAndSendTransaction', 'solana:signTransaction'] };
    const wallet = {
      version,
      name: 'TT E2E Wallet',
      icon: 'data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxIiBoZWlnaHQ9IjEiLz4=',
      chains,
      accounts: [],
      features: {
        'standard:connect': {
          version,
          connect: async () => {
            wallet.accounts = [account];
            (listeners.change ?? []).forEach((f) => f({ accounts: wallet.accounts }));
            return { accounts: wallet.accounts };
          },
        },
        'standard:disconnect': { version, disconnect: async () => { wallet.accounts = []; } },
        'standard:events': {
          version,
          on: (event, fn) => {
            (listeners[event] ??= []).push(fn);
            return () => { listeners[event] = listeners[event].filter((f) => f !== fn); };
          },
        },
        'solana:signAndSendTransaction': {
          version,
          supportedTransactionVersions: ['legacy', 0],
          signAndSendTransaction: async (...inputs) => {
            const out = [];
            for (const i of inputs) out.push({ signature: new Uint8Array(await window.__ttSignAndSend(Array.from(i.transaction))) });
            return out;
          },
        },
        'solana:signTransaction': {
          version,
          supportedTransactionVersions: ['legacy', 0],
          signTransaction: async (...inputs) => {
            const out = [];
            for (const i of inputs) out.push({ signedTransaction: new Uint8Array(await window.__ttSign(Array.from(i.transaction))) });
            return out;
          },
        },
      },
    };
    const register = (api) => api.register(wallet);
    window.addEventListener('wallet-standard:app-ready', (e) => register(e.detail));
    const ev = new Event('wallet-standard:register-wallet', { bubbles: false, cancelable: false, composed: false });
    Object.defineProperty(ev, 'detail', { value: register });
    window.dispatchEvent(ev);
  }, { address, publicKey: Array.from(kp.publicKey) });

  return address;
}

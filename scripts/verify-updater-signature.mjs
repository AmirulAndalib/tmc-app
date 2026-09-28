#!/usr/bin/env node
/**
 * Check every updater signature under a directory against the public key the
 * app is compiled with, before anything is published.
 *
 *   node scripts/verify-updater-signature.mjs <dir>
 *
 * The key is `TMC_UPDATER_PUBKEY` from the environment, else
 * `src-tauri/updater.pub` — exactly what `src-tauri/build.rs` compiles in.
 *
 * WHY THIS EXISTS: the private key is a GitHub secret and the public key is a
 * file in the repository, and nothing else ties the two together. If they are
 * not a pair, every release still builds, signs and uploads perfectly — and
 * every installed app refuses every update, after the download, with an error
 * nobody can diagnose from outside. This makes that a failed release job.
 *
 * Minisign, as Tauri uses it: the `.sig` file is base64 of a minisign
 * signature file whose second line is base64 of `ED` (pre-hashed Ed25519), the
 * 8-byte key id and the 64-byte signature over BLAKE2b-512 of the file. The
 * key is base64 of a minisign public-key file whose second line is base64 of
 * `Ed`, the same key id and the 32-byte public key.
 */
import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const b64 = (text) => Buffer.from(text.trim(), 'base64')

/** The payload line of a base64-wrapped minisign file. */
function payload(wrapped) {
    const lines = b64(wrapped)
        .toString('utf8')
        .split('\n')
        .map((l) => l.trim())
        .filter((l) => l && !l.startsWith('untrusted comment:'))

    if (!lines[0]) throw new Error('not a minisign file')

    return b64(lines[0])
}

export function parsePublicKey(wrapped) {
    const raw = payload(wrapped)

    if (raw.length !== 42 || raw.subarray(0, 2).toString() !== 'Ed')
        throw new Error('not a minisign Ed25519 public key')

    return {
        keyId: raw.subarray(2, 10),
        key: crypto.createPublicKey({
            key: {
                kty: 'OKP',
                crv: 'Ed25519',
                x: raw.subarray(10).toString('base64url'),
            },
            format: 'jwk',
        }),
    }
}

/** True when `sigText` (the `.sig` file's content) signs `data` under `pub`. */
export function verify(pub, data, sigText) {
    const raw = payload(sigText)

    if (raw.length !== 74) return false

    const algorithm = raw.subarray(0, 2).toString()
    const keyId = raw.subarray(2, 10)
    const signature = raw.subarray(10)

    if (!keyId.equals(pub.keyId)) return false

    const message =
        algorithm === 'ED'
            ? crypto.createHash('blake2b512').update(data).digest()
            : data

    return crypto.verify(null, message, pub.key, signature)
}

const SKIP = new Set(['deps', 'build', 'incremental', '.fingerprint', 'examples'])

function sigsUnder(dir) {
    const out = []

    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name)

        // Cargo's own trees are enormous and never hold a bundle.
        if (entry.isDirectory() && !SKIP.has(entry.name))
            out.push(...sigsUnder(full))
        else if (entry.name.endsWith('.sig')) out.push(full)
    }

    return out
}

function main() {
    const dir = process.argv[2]
    if (!dir) throw new Error('Say which directory to check.')

    const here = path.dirname(fileURLToPath(import.meta.url))
    const keyText =
        (process.env.TMC_UPDATER_PUBKEY ?? '').trim() ||
        fs.readFileSync(path.join(here, '..', 'src-tauri', 'updater.pub'), 'utf8')

    const pub = parsePublicKey(keyText)
    const sigs = sigsUnder(dir).filter((s) => fs.existsSync(s.slice(0, -4)))

    if (sigs.length === 0) throw new Error(`No signed artifacts under ${dir}.`)

    let bad = 0

    for (const sig of sigs) {
        const ok = verify(
            pub,
            fs.readFileSync(sig.slice(0, -4)),
            fs.readFileSync(sig, 'utf8')
        )
        console.log(
            `${ok ? 'ok ' : 'BAD'}  ${path.relative(dir, sig.slice(0, -4))}`
        )
        if (!ok) bad++
    }

    if (bad > 0)
        throw new Error(
            `${bad} signature(s) do not verify against the compiled-in public key.\n` +
                'TAURI_SIGNING_PRIVATE_KEY and src-tauri/updater.pub (or TMC_UPDATER_PUBKEY)\n' +
                'are not a pair. Every installed app would refuse this release.'
        )
}

if (
    process.argv[1] &&
    fileURLToPath(import.meta.url) === path.resolve(process.argv[1])
) {
    try {
        main()
    } catch (error) {
        console.error(
            `\n${error instanceof Error ? error.message : String(error)}\n`
        )
        process.exit(1)
    }
}

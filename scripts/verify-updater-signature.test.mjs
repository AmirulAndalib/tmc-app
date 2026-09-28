import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import { parsePublicKey, verify } from './verify-updater-signature.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const read = (p) => fs.readFileSync(path.join(here, p))

/*
 * `fixtures/signed.txt.sig` was made by `tauri signer sign` with the release
 * key, so this also proves `src-tauri/updater.pub` — the key every build
 * compiles in — is the other half of the key releases are signed with.
 */
describe('updater signature verification', () => {
    const pub = parsePublicKey(read('../src-tauri/updater.pub').toString())
    const sig = read('fixtures/signed.txt.sig').toString()

    it('accepts what the release key signed', () => {
        expect(verify(pub, read('fixtures/signed.txt'), sig)).toBe(true)
    })

    it('refuses a changed file', () => {
        expect(verify(pub, Buffer.from('hello!\n'), sig)).toBe(false)
    })

    it('refuses something that is not a key', () => {
        expect(() =>
            parsePublicKey(Buffer.from('nope').toString('base64'))
        ).toThrow()
    })
})

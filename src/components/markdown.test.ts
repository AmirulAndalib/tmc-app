/**
 * The one scheme check between an untrusted mod description and the system
 * browser.
 *
 * The link it approves is handed to `openUrl`, whose capability allows
 * `https://*` and nothing else. The two have to agree: a scheme approved here
 * but refused there is a link that renders, looks clickable, and does nothing.
 */

import { describe, expect, it } from 'vitest'

import { safeHref } from './markdown'

describe('safeHref', () => {
    it('passes https through, normalised', () => {
        expect(safeHref('https://example.com/a?b=c#d')).toBe(
            'https://example.com/a?b=c#d'
        )
        expect(safeHref('HTTPS://Example.com')).toBe('https://example.com/')
    })

    it('refuses http, because the opener capability would refuse it silently', () => {
        expect(safeHref('http://example.com/')).toBeNull()
    })

    it('refuses every scheme that is script, content or a local handler', () => {
        for (const raw of [
            'javascript:alert(1)',
            'JaVaScRiPt:alert(1)',
            'data:text/html,<script>alert(1)</script>',
            'file:///etc/passwd',
            'steam://connect/1.2.3.4:27015',
            'tmc://view/1',
            'vbscript:msgbox(1)',
        ])
            expect(safeHref(raw)).toBeNull()
    })

    it('refuses what is not an absolute URL at all', () => {
        expect(safeHref('/relative/path')).toBeNull()
        expect(safeHref('//example.com/proto-relative')).toBeNull()
        expect(safeHref('not a url')).toBeNull()
        expect(safeHref('')).toBeNull()
    })
})

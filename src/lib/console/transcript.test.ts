import { describe, expect, it } from 'vitest'

import {
    appendLine,
    appendOutput,
    emptyTranscript,
    MAX_LINES,
    plainText,
    tokenize,
    initialAnsi,
} from './transcript'

const texts = (t: ReturnType<typeof emptyTranscript>) =>
    t.lines.map((l) => l.spans.map((s) => s.text).join(''))

describe('colour', () => {
    it('maps the basic colours onto the theme and resets', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', '\u001b[31mbad\u001b[0m ok \u001b[1;32mgood\u001b[0m\n')

        const spans = t.lines[0]?.spans ?? []

        expect(spans.map((s) => [s.text, s.style.tone, s.style.bold])).toEqual([
            ['bad', 'danger', false],
            [' ok ', null, false],
            ['good', 'success', true],
        ])
    })

    it('carries colour across chunks and lines, like a terminal', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stderr', '\u001b[33mwarn')
        appendOutput(t, 'stderr', ' more\nnext\u001b[0m\n')

        expect(t.lines.map((l) => l.spans.map((s) => s.style.tone))).toEqual([
            ['warning', 'warning'],
            ['warning'],
        ])
    })

    it('holds back an escape cut in half', () => {
        const a = tokenize(initialAnsi(), 'x\u001b[3')
        const b = tokenize(a.state, '1mred')

        expect(a.state.pending).toBe('\u001b[3')
        expect(b.pieces).toEqual([
            { kind: 'text', text: 'red', style: expect.objectContaining({ tone: 'danger' }) },
        ])
    })

    it('skips extended colour operands and drops other sequences', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', '\u001b[38;5;196mA\u001b[2K\u001b[1A\u001b]0;title\u0007B\u001b[0m\n')

        expect(texts(t)).toEqual(['AB'])
        expect(t.lines[0]?.spans[0]?.style.tone).toBeNull()
    })

    it('strips stray control characters', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', 'a\u0000b\u0007c\td\n')

        expect(texts(t)).toEqual(['abc\td'])
    })
})

describe('lines', () => {
    it('keeps a prompt without a newline open, and finishes it on the next write', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stderr', 'API token: ')

        expect(t.lines[0]?.open).toBe(true)

        appendOutput(t, 'stderr', 'ok\n')

        expect(texts(t)).toEqual(['API token: ok'])
        expect(t.lines[0]?.open).toBe(false)
    })

    it('ends the open line when the other stream writes', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', 'partial')
        appendOutput(t, 'stderr', 'oops\n')

        expect(texts(t)).toEqual(['partial', 'oops'])
        expect(t.lines.map((l) => l.kind)).toEqual(['stdout', 'stderr'])
    })

    it('treats CRLF as a line ending and a bare CR as a redraw', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', 'one\r\ntwo\r')
        appendOutput(t, 'stdout', '10%\r50%\rdone\n')

        expect(texts(t)).toEqual(['one', 'done'])
    })

    it('keeps blank lines', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', 'a\n\nb\n')

        expect(texts(t)).toEqual(['a', '', 'b'])
    })

    it('caps the transcript', () => {
        const t = emptyTranscript()

        appendOutput(t, 'stdout', Array.from({ length: MAX_LINES + 50 }, (_, i) => `l${i}`).join('\n') + '\n')

        expect(t.lines).toHaveLength(MAX_LINES)
        expect(texts(t)[0]).toBe('l50')
    })

    it('copies as plain text with the command prefixed', () => {
        const t = emptyTranscript()

        appendLine(t, 'cmd', 'mod list')
        appendOutput(t, 'stdout', '\u001b[32mid\u001b[0m name\n')

        expect(plainText(t)).toBe('$ tmc mod list\nid name')
    })
})

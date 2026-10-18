/**
 * The Console's transcript: what `tmc` printed, as lines of coloured spans.
 *
 * Pure, so it is tested without a DOM. Two jobs live here because they are the
 * same problem seen from two sides: turning a byte-ish stream into lines
 * (carriage returns, partial lines, a prompt with no newline) and turning ANSI
 * colour into spans.
 */

/** Which of the app's own text colours a terminal colour maps to. */
export type Tone = 'danger' | 'success' | 'warning' | 'accent' | 'muted' | null

export type Style = {
    tone: Tone
    bold: boolean
    dim: boolean
    italic: boolean
    underline: boolean
}

export type Span = { text: string; style: Style }

export const PLAIN: Style = {
    tone: null,
    bold: false,
    dim: false,
    italic: false,
    underline: false,
}

/** Colour is carried over chunk and line boundaries, as a terminal does. */
export type AnsiState = {
    style: Style
    /** An escape sequence cut off at the end of the last chunk. */
    pending: string
}

export function initialAnsi(): AnsiState {
    return { style: PLAIN, pending: '' }
}

/**
 * Terminal colours collapse onto the theme: a colour that is not in the shared
 * theme is a colour that is wrong in the other theme. Red/green/yellow keep
 * their meaning; blue, magenta and cyan are all the accent; black, white and
 * grey are the foreground.
 */
function toneOf(code: number): Tone {
    switch (code % 10) {
        case 1:
            return 'danger'
        case 2:
            return 'success'
        case 3:
            return 'warning'
        case 4:
        case 5:
        case 6:
            return 'accent'
        default:
            return null
    }
}

function applySgr(style: Style, params: number[]): Style {
    let next = { ...style }

    // `ESC[m` is `ESC[0m`.
    const list = params.length === 0 ? [0] : params

    for (let i = 0; i < list.length; i++) {
        const p = list[i] ?? 0

        if (p === 0) next = { ...PLAIN }
        else if (p === 1) next.bold = true
        else if (p === 2) next.dim = true
        else if (p === 3) next.italic = true
        else if (p === 4) next.underline = true
        else if (p === 22) {
            next.bold = false
            next.dim = false
        } else if (p === 23) next.italic = false
        else if (p === 24) next.underline = false
        else if ((p >= 30 && p <= 37) || (p >= 90 && p <= 97)) next.tone = toneOf(p)
        else if (p === 39) next.tone = null
        else if (p === 38 || p === 48) {
            // Extended colour: `;5;n` or `;2;r;g;b`. Skipped, but its operands
            // must not be read as separate codes.
            i += list[i + 1] === 5 ? 2 : list[i + 1] === 2 ? 4 : 0
        }
        // Backgrounds, blink, reverse and the rest are ignored on purpose.
    }

    return next
}

/** Longest escape sequence held back waiting for its end. */
const MAX_PENDING = 64

type Piece =
    { kind: 'text'; text: string; style: Style } | { kind: 'cr' } | { kind: 'lf' }

/**
 * Split a chunk into text, line breaks and carriage returns, applying colour
 * codes to the state and dropping every other escape sequence.
 */
export function tokenize(
    state: AnsiState,
    input: string
): { pieces: Piece[]; state: AnsiState } {
    const pieces: Piece[] = []
    let style = state.style
    let buf = ''
    const src = state.pending + input
    let pending = ''

    const flush = () => {
        if (buf) pieces.push({ kind: 'text', text: buf, style })

        buf = ''
    }

    for (let i = 0; i < src.length; i++) {
        const ch = src.charAt(i)

        if (ch === '\u001b') {
            const rest = src.slice(i)
            // eslint-disable-next-line no-control-regex -- matching terminal escapes is the point
            const csi = /^\u001b\[([0-9;:?]*)([@-~])/.exec(rest)

            if (csi) {
                if (csi[2] === 'm') {
                    flush()
                    style = applySgr(
                        style,
                        (csi[1] ?? '')
                            .split(/[;:]/)
                            .filter((s) => s !== '')
                            .map(Number)
                    )
                }

                i += csi[0].length - 1

                continue
            }

            // OSC (title, hyperlink) up to BEL or ST: dropped.
            // eslint-disable-next-line no-control-regex -- matching terminal escapes is the point
            const osc = /^\u001b\][^\u0007\u001b]*(\u0007|\u001b\\)/.exec(rest)

            if (osc) {
                i += osc[0].length - 1

                continue
            }

            // Looks like the start of one that has not finished arriving.
            if (
                rest.length < MAX_PENDING &&
                // eslint-disable-next-line no-control-regex -- matching terminal escapes is the point
                /^\u001b(\[[0-9;:?]*|\][^\u0007]*\u001b?)?$/.test(rest)
            ) {
                pending = rest

                break
            }

            // A lone ESC or a two-character sequence: dropped.
            continue
        }

        if (ch === '\r') {
            flush()
            pieces.push({ kind: 'cr' })
        } else if (ch === '\n') {
            flush()
            pieces.push({ kind: 'lf' })
        } else if (ch === '\u0008') {
            // Backspace: not worth emulating.
        } else if (ch < ' ' && ch !== '\t') {
            // Other control characters would render as boxes.
        } else {
            buf += ch
        }
    }

    flush()

    return { pieces, state: { style, pending } }
}

export type LineKind = 'cmd' | 'stdout' | 'stderr' | 'input' | 'note'

export type Line = {
    id: number
    kind: LineKind
    spans: Span[]
    /** Bumped on every change, so a memoised row knows to redraw. */
    rev: number
    /** No newline has arrived yet: a prompt, or a progress line being redrawn. */
    open: boolean
}

export type Transcript = {
    lines: Line[]
    ansi: { stdout: AnsiState; stderr: AnsiState }
    nextId: number
    /** The last output ended in a bare `\r`: the next text replaces the open line. */
    cr: boolean
}

/** Oldest lines are dropped past this: a long listing must not grow forever. */
export const MAX_LINES = 5000

export function emptyTranscript(): Transcript {
    return {
        lines: [],
        ansi: { stdout: initialAnsi(), stderr: initialAnsi() },
        nextId: 1,
        cr: false,
    }
}

function push(t: Transcript, kind: LineKind, spans: Span[], open: boolean): Line {
    const line: Line = { id: t.nextId++, kind, spans, rev: 0, open }

    t.lines.push(line)

    if (t.lines.length > MAX_LINES) t.lines.splice(0, t.lines.length - MAX_LINES)

    return line
}

/** A whole line that is not process output: the command, a note, an echo. */
export function appendLine(t: Transcript, kind: LineKind, text: string): void {
    closeOpen(t)
    t.cr = false
    push(t, kind, text ? [{ text, style: PLAIN }] : [], false)
}

function closeOpen(t: Transcript): void {
    const last = t.lines[t.lines.length - 1]

    if (last?.open) {
        last.open = false
        last.rev++
    }
}

/**
 * Add process output. Mutates `t` and returns it, because a run can append
 * hundreds of chunks a second and copying a 5000-line array for each would be
 * the slowest thing on the screen; the caller bumps a version to re-render.
 *
 * A carriage return that is not part of `\r\n` sends the cursor to the start
 * of the line, so what follows REPLACES it. That is how a progress line
 * redraws itself, and rendering it as appended text would print every step.
 */
export function appendOutput(
    t: Transcript,
    stream: 'stdout' | 'stderr',
    text: string
): Transcript {
    const { pieces, state } = tokenize(t.ansi[stream], text)

    t.ansi[stream] = state

    let last = t.lines[t.lines.length - 1]

    // The other stream wrote into a line still open: that line is finished.
    if (last?.open && last.kind !== stream) {
        last.open = false
        last.rev++
        last = undefined
        t.cr = false
    }

    let rewind = t.cr

    for (const piece of pieces) {
        if (piece.kind === 'cr') {
            rewind = true

            continue
        }

        if (piece.kind === 'lf') {
            // `\r\n`: the rewind was only a line ending.
            rewind = false

            if (!last?.open) push(t, stream, [], false)
            else {
                last.open = false
                last.rev++
            }

            last = undefined

            continue
        }

        if (!last?.open) {
            last = push(t, stream, [], true)
        } else if (rewind) {
            last.spans = []
        }

        rewind = false
        last.rev++
        last.spans.push({ text: piece.text, style: piece.style })
    }

    t.cr = rewind

    return t
}

/** The transcript as plain text, for the copy button. */
export function plainText(t: Transcript): string {
    return t.lines
        .map((l) => {
            const body = l.spans.map((s) => s.text).join('')

            return l.kind === 'cmd' ? `$ tmc ${body}` : body
        })
        .join('\n')
}

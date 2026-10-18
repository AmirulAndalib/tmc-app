import { describe, expect, it } from 'vitest'

import { MAX_HISTORY, pushHistory, walkHistory } from './history'

describe('history', () => {
    it('ignores blanks and immediate repeats', () => {
        expect(pushHistory([], '  ')).toEqual([])
        expect(pushHistory(['a'], 'a')).toEqual(['a'])
        expect(pushHistory(['a'], ' b ')).toEqual(['a', 'b'])
        expect(pushHistory(['a', 'b'], 'a')).toEqual(['a', 'b', 'a'])
    })

    it('is capped, dropping the oldest', () => {
        let list: string[] = []

        for (let i = 0; i < MAX_HISTORY + 5; i++) list = pushHistory(list, `c${i}`)

        expect(list).toHaveLength(MAX_HISTORY)
        expect(list[0]).toBe('c5')
    })

    it('walks up and down and returns to the draft', () => {
        const list = ['a', 'b']

        let s = walkHistory(list, 2, 'up', 'typing')
        expect(s).toEqual({ cursor: 1, text: 'b' })

        s = walkHistory(list, s.cursor, 'up', 'typing')
        s = walkHistory(list, s.cursor, 'up', 'typing')
        expect(s).toEqual({ cursor: 0, text: 'a' })

        s = walkHistory(list, 0, 'down', 'typing')
        s = walkHistory(list, s.cursor, 'down', 'typing')
        expect(s).toEqual({ cursor: 2, text: 'typing' })
    })
})

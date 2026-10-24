import { useState, type ReactNode } from 'react'
import { FiCheck, FiSearch } from 'react-icons/fi'

export type FacetOption = {
    id: string
    label: string
    count?: number
    leading?: ReactNode
    indent?: boolean
}

/**
 * A pick list with a leading picture and a count on every row.
 *
 * Rows are 40px tall so a thumb lands on one; the search box only appears once
 * the list is long enough to need it. Single mode behaves as a radio group.
 */
export default function FacetList({
    label,
    options,
    selected,
    onToggle,
    multiple = true,
    searchAt = 8,
    maxHeightClass = 'max-h-64',
}: {
    label: string
    options: FacetOption[]
    selected: string[]
    onToggle: (id: string) => void
    multiple?: boolean
    searchAt?: number
    maxHeightClass?: string
}) {
    const [q, setQ] = useState('')

    const needle = q.trim().toLowerCase()
    const shown = needle
        ? options.filter((o) => o.label.toLowerCase().includes(needle))
        : options

    return (
        <div className="flex flex-col gap-1.5">
            <span className="text-[0.7rem] text-muted">{label}</span>

            <div className="overflow-hidden rounded-lg border border-border bg-surface">
                {options.length >= searchAt && (
                    <div className="relative border-b border-border">
                        <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                        <input
                            value={q}
                            onChange={(e) => setQ(e.target.value)}
                            placeholder={`Find ${label.toLowerCase()}`}
                            aria-label={`Find ${label.toLowerCase()}`}
                            className="w-full bg-transparent py-2 pl-8 pr-2 text-base outline-none sm:text-sm"
                        />
                    </div>
                )}

                <ul
                    role={multiple ? 'group' : 'radiogroup'}
                    aria-label={label}
                    className={`${maxHeightClass} overflow-y-auto overscroll-contain`}
                >
                    {shown.map((o) => {
                        const on = selected.includes(o.id)

                        return (
                            <li key={o.id}>
                                <button
                                    type="button"
                                    role={multiple ? 'checkbox' : 'radio'}
                                    aria-checked={on}
                                    onClick={() => onToggle(o.id)}
                                    className={`flex min-h-10 w-full items-center gap-2.5 px-2.5 text-left text-sm transition-colors ${
                                        o.indent ? 'pl-7' : ''
                                    } ${
                                        on
                                            ? 'bg-accent/15 text-foreground'
                                            : 'hover:bg-surface-2'
                                    }`}
                                >
                                    {o.leading}
                                    <span className="min-w-0 flex-1 truncate">
                                        {o.label}
                                    </span>
                                    {o.count !== undefined && o.count > 0 && (
                                        <span className="shrink-0 text-xs tabular-nums text-muted">
                                            {o.count.toLocaleString()}
                                        </span>
                                    )}
                                    <span
                                        aria-hidden
                                        className={`flex size-4 shrink-0 items-center justify-center rounded-full ${
                                            on
                                                ? 'bg-accent text-accent-foreground'
                                                : 'border border-border'
                                        }`}
                                    >
                                        {on && <FiCheck className="size-3" />}
                                    </span>
                                </button>
                            </li>
                        )
                    })}

                    {shown.length === 0 && (
                        <li className="px-3 py-3 text-xs text-muted">
                            Nothing matches.
                        </li>
                    )}
                </ul>
            </div>
        </div>
    )
}

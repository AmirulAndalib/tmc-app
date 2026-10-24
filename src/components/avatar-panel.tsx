import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
    FiArrowDown,
    FiArrowUp,
    FiExternalLink,
    FiHeart,
    FiPackage,
    FiPlus,
    FiRefreshCw,
    FiSave,
    FiSmile,
    FiUser,
    FiX,
    FiZap,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { AvatarDocT, AvatarSchemaResponseT } from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'
import { openExternally } from '~/lib/external'
import { messageOf } from '~/lib/ipc'

type TabKey = 'avatar' | 'EMOTE' | 'POSE' | 'GESTURE' | 'WEARABLES'

const TABS: { key: TabKey; label: string; icon: typeof FiUser }[] = [
    { key: 'avatar', label: 'Avatar', icon: FiUser },
    { key: 'EMOTE', label: 'Emotes', icon: FiSmile },
    { key: 'POSE', label: 'Poses', icon: FiHeart },
    { key: 'GESTURE', label: 'Gestures', icon: FiZap },
    { key: 'WEARABLES', label: 'Wearables', icon: FiPackage },
]

const KEY_HINT: Record<'EMOTE' | 'POSE' | 'GESTURE', string> = {
    EMOTE: ',',
    POSE: '.',
    GESTURE: '/',
}

/** A stable hue from a string, so a part is the same colour every draw. */
function hueOf(text: string): number {
    let h = 0

    for (const ch of text) h = (h * 31 + ch.charCodeAt(0)) % 360

    return h
}

function Figure({ doc, twoD }: { doc: AvatarDocT; twoD: string | null }) {
    if (twoD) {
        return (
            <img
                src={twoD}
                alt="Your painted avatar"
                className="h-48 w-48 rounded-xl bg-background object-contain"
            />
        )
    }

    const top = `#${doc.colours.top?.[0] ?? '4c6ef5'}`
    const legs = `#${doc.colours.legs?.[0] ?? '364152'}`
    const face = `hsl(${hueOf(doc.parts.face ?? 'face')} 45% 62%)`

    return (
        <svg
            viewBox="0 0 120 160"
            role="img"
            aria-label="Your avatar, drawn from its colours"
            className="h-48 w-40 rounded-xl bg-background"
        >
            <ellipse
                cx="60"
                cy="150"
                rx="30"
                ry="5"
                fill="currentColor"
                opacity="0.12"
            />
            <rect x="42" y="92" width="14" height="52" rx="6" fill={legs} />
            <rect x="64" y="92" width="14" height="52" rx="6" fill={legs} />
            <rect x="34" y="52" width="52" height="52" rx="14" fill={top} />
            <rect x="22" y="56" width="14" height="40" rx="7" fill={top} />
            <rect x="84" y="56" width="14" height="40" rx="7" fill={top} />
            <circle cx="60" cy="32" r="22" fill={face} />
            <circle cx="52" cy="30" r="2.5" fill="#1a1a1a" />
            <circle cx="68" cy="30" r="2.5" fill="#1a1a1a" />
            <path
                d="M52 40 Q60 46 68 40"
                stroke="#1a1a1a"
                strokeWidth="2"
                fill="none"
                strokeLinecap="round"
            />
        </svg>
    )
}

function SlotEditor({
    schema,
    doc,
    onChange,
}: {
    schema: AvatarSchemaResponseT
    doc: AvatarDocT
    onChange: (next: AvatarDocT) => void
}) {
    const slots = useMemo(
        () =>
            [...schema.slots]
                .sort((a, b) => a.layer - b.layer)
                .map((slot) => ({
                    slot,
                    parts: schema.parts.filter(
                        (p) => p.slot === slot.id && !p.retired
                    ),
                })),
        [schema]
    )

    return (
        <div className="flex min-w-0 flex-1 flex-col gap-4">
            {slots.map(({ slot, parts }) => {
                const chosen = doc.parts[slot.id] ?? ''
                const channels =
                    parts.find((p) => p.id === chosen)?.colour_channels ?? 0

                return (
                    <div key={slot.id} className="flex flex-col gap-1.5">
                        <span className="text-xs font-medium text-muted">
                            {slot.display_name}
                        </span>

                        <div className="-mx-1 flex gap-1.5 overflow-x-auto px-1 pb-1">
                            {!slot.required && (
                                <button
                                    type="button"
                                    aria-pressed={chosen === ''}
                                    onClick={() => {
                                        const parts2 = { ...doc.parts }

                                        delete parts2[slot.id]
                                        onChange({ ...doc, parts: parts2 })
                                    }}
                                    className={`shrink-0 rounded-full border px-3 py-1.5 text-xs ${
                                        chosen === ''
                                            ? 'border-accent bg-accent/15'
                                            : 'border-border'
                                    }`}
                                >
                                    None
                                </button>
                            )}

                            {parts.map((p) => (
                                <button
                                    key={p.id}
                                    type="button"
                                    aria-pressed={chosen === p.id}
                                    onClick={() =>
                                        onChange({
                                            ...doc,
                                            parts: {
                                                ...doc.parts,
                                                [slot.id]: p.id,
                                            },
                                        })
                                    }
                                    className={`shrink-0 rounded-full border px-3 py-1.5 text-xs ${
                                        chosen === p.id
                                            ? 'border-accent bg-accent/15'
                                            : 'border-border hover:bg-surface-2'
                                    }`}
                                >
                                    {p.display_name}
                                </button>
                            ))}
                        </div>

                        {channels > 0 && (
                            <div className="flex items-center gap-2">
                                {Array.from({ length: channels }, (_, i) => (
                                    <input
                                        key={i}
                                        type="color"
                                        aria-label={`${slot.display_name} colour ${i + 1}`}
                                        value={`#${doc.colours[slot.id]?.[i] ?? '808080'}`}
                                        onChange={(e) => {
                                            const next = [
                                                ...(doc.colours[slot.id] ?? []),
                                            ]

                                            while (next.length < channels)
                                                next.push('808080')
                                            next[i] = e.target.value.replace(
                                                '#',
                                                ''
                                            )
                                            onChange({
                                                ...doc,
                                                colours: {
                                                    ...doc.colours,
                                                    [slot.id]: next,
                                                },
                                            })
                                        }}
                                        className="h-9 w-12 cursor-pointer rounded-md border border-border bg-transparent p-0.5"
                                    />
                                ))}
                            </div>
                        )}
                    </div>
                )
            })}
        </div>
    )
}

function ActionTab({
    kind,
    manageUrl,
}: {
    kind: 'EMOTE' | 'POSE' | 'GESTURE'
    manageUrl: string | null
}) {
    const client = useQueryClient()
    const noun =
        kind === 'EMOTE' ? 'emotes' : kind === 'POSE' ? 'poses' : 'gestures'

    const list = useQuery({
        queryKey: ['avatar', 'actions'],
        queryFn: () => api.avatarActions(),
    })

    const save = useMutation({
        mutationFn: (ids: number[]) => api.avatarActionsSave(kind, ids),
        onSuccess: () => {
            void client.invalidateQueries({ queryKey: ['avatar'] })
        },
    })

    if (list.error) {
        return <p className="text-sm text-danger">{messageOf(list.error)}</p>
    }

    if (!list.data) {
        return <div className="h-32 animate-pulse rounded-lg bg-surface-2" />
    }

    const max = list.data.max
    const all = list.data.actions.filter((a) => a.kind === kind)
    const equipped = all
        .filter((a) => a.equipped)
        .sort((a, b) => (a.slot ?? 0) - (b.slot ?? 0))
    const available = all.filter((a) => !a.equipped)
    const ids = equipped.map((a) => a.id)
    const busy = save.isPending

    const move = (i: number, by: -1 | 1) => {
        const next = [...ids]
        const [x] = next.splice(i, 1)
        if (x !== undefined) next.splice(i + by, 0, x)
        save.mutate(next)
    }

    return (
        <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
                <h3 className="flex items-center justify-between text-xs font-medium uppercase tracking-wide text-muted">
                    <span>Equipped</span>
                    <span className="tabular-nums normal-case tracking-normal">
                        {equipped.length} / {max}
                    </span>
                </h3>

                {equipped.length > 0 ? (
                    <ol className="flex flex-col gap-2">
                        {equipped.map((a, i) => (
                            <li
                                key={a.id}
                                className="flex items-center gap-1 rounded-lg border border-border bg-surface-2 py-1.5 pl-2.5 pr-1"
                            >
                                <span className="mr-1 flex size-7 shrink-0 items-center justify-center rounded-md bg-accent/15 text-xs font-semibold tabular-nums">
                                    {i + 1}
                                </span>
                                <span className="min-w-0 flex-1 truncate text-sm">
                                    {a.name}
                                </span>
                                <IconBtn
                                    label={`Move ${a.name} up`}
                                    disabled={busy || i === 0}
                                    onClick={() => move(i, -1)}
                                >
                                    <FiArrowUp className="size-4" />
                                </IconBtn>
                                <IconBtn
                                    label={`Move ${a.name} down`}
                                    disabled={busy || i === equipped.length - 1}
                                    onClick={() => move(i, 1)}
                                >
                                    <FiArrowDown className="size-4" />
                                </IconBtn>
                                <IconBtn
                                    label={`Remove ${a.name}`}
                                    disabled={busy}
                                    onClick={() =>
                                        save.mutate(ids.filter((x) => x !== a.id))
                                    }
                                >
                                    <FiX className="size-4" />
                                </IconBtn>
                            </li>
                        ))}
                    </ol>
                ) : (
                    <p className="rounded-lg border border-dashed border-border px-4 py-5 text-center text-sm text-muted">
                        Nothing equipped. Games use their built-in {noun}.
                    </p>
                )}
            </div>

            <div className="flex flex-col gap-2">
                <h3 className="text-xs font-medium uppercase tracking-wide text-muted">
                    Available
                </h3>

                {available.length > 0 ? (
                    <ul className="grid grid-cols-1 gap-2 xs:grid-cols-2">
                        {available.map((a) => (
                            <li key={a.id}>
                                <button
                                    type="button"
                                    disabled={busy || equipped.length >= max}
                                    onClick={() => save.mutate([...ids, a.id])}
                                    className="flex min-h-11 w-full items-center gap-2 rounded-lg border border-border px-3 py-2 text-left text-sm hover:bg-surface-2 disabled:opacity-50"
                                >
                                    <FiPlus className="size-4 shrink-0 text-accent" />
                                    <span className="min-w-0 flex-1 truncate">
                                        {a.name}
                                    </span>
                                    {a.builtin && (
                                        <span className="shrink-0 rounded-full border border-border px-2 py-0.5 text-[0.65rem] text-muted">
                                            Built in
                                        </span>
                                    )}
                                </button>
                            </li>
                        ))}
                    </ul>
                ) : (
                    <p className="text-sm text-muted">
                        Everything available is equipped.
                    </p>
                )}
            </div>

            {save.error && (
                <p className="text-xs text-danger">{messageOf(save.error)}</p>
            )}

            <p className="text-xs text-muted">
                In game, press{' '}
                <kbd className="rounded border border-border px-1">
                    {KEY_HINT[kind]}
                </kbd>{' '}
                to play the first one; press again to cycle.
            </p>

            {manageUrl && (
                <button
                    type="button"
                    onClick={() => void openExternally({ webUrl: manageUrl })}
                    className="flex w-fit items-center gap-1.5 rounded-lg border border-border px-3 py-2 text-sm"
                >
                    <FiExternalLink className="size-3.5 shrink-0" />
                    Upload your own on the website
                </button>
            )}
        </div>
    )
}

const SLOT_LABEL: Record<string, string> = {
    hat: 'Hat',
    face: 'Face',
    neck: 'Neck',
    back: 'Back',
    waist: 'Waist',
    hand_right: 'Right hand',
    hand_left: 'Left hand',
}

/** Where each slot sits on the 120x160 stick figure, for the worn-slot markers. */
const SLOT_SPOT: Record<string, [number, number]> = {
    hat: [60, 8],
    face: [60, 34],
    neck: [60, 52],
    back: [100, 70],
    waist: [60, 100],
    hand_right: [16, 100],
    hand_left: [104, 100],
}

/** The member's figure with a marker on each slot that has something on it. */
function WornFigure({ worn }: { worn: Set<string> }) {
    return (
        <svg
            viewBox="0 0 120 160"
            role="img"
            aria-label={`Wearing in: ${[...worn].map((s) => SLOT_LABEL[s] ?? s).join(', ') || 'nothing'}`}
            className="h-40 w-32 shrink-0 rounded-xl bg-background text-muted"
        >
            <circle cx="60" cy="32" r="20" fill="none" stroke="currentColor" opacity="0.4" strokeWidth="2" />
            <rect x="36" y="54" width="48" height="50" rx="12" fill="none" stroke="currentColor" opacity="0.4" strokeWidth="2" />
            <rect x="44" y="104" width="12" height="46" rx="5" fill="none" stroke="currentColor" opacity="0.4" strokeWidth="2" />
            <rect x="64" y="104" width="12" height="46" rx="5" fill="none" stroke="currentColor" opacity="0.4" strokeWidth="2" />
            {Object.entries(SLOT_SPOT).map(([slot, [x, y]]) => (
                <circle
                    key={slot}
                    cx={x}
                    cy={y}
                    r={worn.has(slot) ? 6 : 3.5}
                    className={worn.has(slot) ? 'fill-accent' : 'fill-current'}
                    opacity={worn.has(slot) ? 1 : 0.35}
                />
            ))}
        </svg>
    )
}

function WearablesTab({ manageUrl }: { manageUrl: string | null }) {
    const client = useQueryClient()
    const list = useQuery({
        queryKey: ['avatar', 'wearables'],
        queryFn: () => api.avatarWearables(),
    })
    const equip = useMutation({
        mutationFn: (v: { id: number; equipped: boolean }) =>
            api.avatarWearableEquip(v.id, v.equipped),
        onSuccess: () => {
            void client.invalidateQueries({ queryKey: ['avatar', 'wearables'] })
        },
    })

    if (list.error) {
        return <p className="text-sm text-danger">{messageOf(list.error)}</p>
    }

    if (!list.data) {
        return <div className="h-32 animate-pulse rounded-lg bg-surface-2" />
    }

    const busy = equip.isPending
    const worn = new Set(
        list.data.wearables.filter((w) => w.equipped).map((w) => w.slot)
    )

    return (
        <div className="flex flex-col gap-4">
            <div className="flex items-center gap-3">
                <WornFigure worn={worn} />
                <p className="text-sm text-muted">
                    Hats, glasses, backpacks and held items hang on your
                    figure in games that support them. One per slot.
                </p>
            </div>

            {list.data.slots.map((slot) => {
                const rows = list.data.wearables.filter((w) => w.slot === slot)
                const on = rows.find((w) => w.equipped)

                return (
                    <div key={slot} className="flex flex-col gap-2">
                        <h3 className="flex items-center justify-between text-xs font-medium uppercase tracking-wide text-muted">
                            <span>{SLOT_LABEL[slot] ?? slot}</span>
                            <span className="normal-case tracking-normal">
                                {on ? on.name : 'Empty'}
                            </span>
                        </h3>

                        {rows.length > 0 ? (
                            <ul className="grid grid-cols-1 gap-2 xs:grid-cols-2">
                                {rows.map((w) => (
                                    <li key={w.id}>
                                        <button
                                            type="button"
                                            aria-pressed={w.equipped}
                                            disabled={busy || (w.locked && !w.equipped)}
                                            onClick={() =>
                                                equip.mutate({
                                                    id: w.id,
                                                    equipped: !w.equipped,
                                                })
                                            }
                                            className={`flex min-h-11 w-full items-center gap-2 rounded-lg border px-3 py-2 text-left text-sm disabled:opacity-50 ${
                                                w.equipped
                                                    ? 'border-accent bg-accent/15'
                                                    : 'border-border hover:bg-surface-2'
                                            }`}
                                        >
                                            <span className="min-w-0 flex-1 truncate">
                                                {w.name}
                                            </span>
                                            {w.locked && (
                                                <span className="shrink-0 rounded-full border border-border px-2 py-0.5 text-[0.65rem] text-muted">
                                                    In the store
                                                </span>
                                            )}
                                            {w.equipped && (
                                                <FiX className="size-4 shrink-0" />
                                            )}
                                        </button>
                                    </li>
                                ))}
                            </ul>
                        ) : (
                            <p className="text-sm text-muted">
                                Nothing for this slot yet.
                            </p>
                        )}
                    </div>
                )
            })}

            {equip.error && (
                <p className="text-xs text-danger">{messageOf(equip.error)}</p>
            )}

            {manageUrl && (
                <button
                    type="button"
                    onClick={() => void openExternally({ webUrl: manageUrl })}
                    className="flex w-fit items-center gap-1.5 rounded-lg border border-border px-3 py-2 text-sm"
                >
                    <FiExternalLink className="size-3.5 shrink-0" />
                    Make your own on the website
                </button>
            )}
        </div>
    )
}

function IconBtn({
    label,
    disabled,
    onClick,
    children,
}: {
    label: string
    disabled: boolean
    onClick: () => void
    children: React.ReactNode
}) {
    return (
        <button
            type="button"
            aria-label={label}
            title={label}
            disabled={disabled}
            onClick={onClick}
            className="flex size-9 shrink-0 items-center justify-center rounded-md hover:bg-surface disabled:opacity-30"
        >
            {children}
        </button>
    )
}

/**
 * The signed-in member's avatar: the figure and its parts, plus the emotes,
 * poses, gestures and wearables they have equipped.
 *
 * The figure is account-wide (a per-server override is the website's), so the
 * panel is the same on every game page. Hidden when the site has avatars off.
 */
export default function AvatarPanel({ siteUrl }: { siteUrl: string | null }) {
    const { status } = useAuth()
    const signedIn = status === 'signedIn'
    const client = useQueryClient()
    const [tab, setTab] = useState<TabKey>('avatar')
    const [draft, setDraft] = useState<AvatarDocT | null>(null)

    const mine = useQuery({
        queryKey: ['avatar', 'me'],
        queryFn: () => api.avatar(),
        enabled: signedIn,
        retry: false,
    })

    const schema = useQuery({
        queryKey: ['avatar', 'schema'],
        queryFn: () => api.avatarSchema(),
        enabled: signedIn,
        staleTime: 60 * 60 * 1000,
        retry: false,
    })

    const save = useMutation({
        mutationFn: (doc: AvatarDocT) => api.avatarSave(doc),
        onSuccess: async () => {
            setDraft(null)
            await client.invalidateQueries({ queryKey: ['avatar', 'me'] })
        },
    })

    const manageUrl = useMemo(() => {
        if (!siteUrl) return null

        try {
            return `${new URL(siteUrl).origin}/dashboard/avatars`
        } catch {
            return null
        }
    }, [siteUrl])

    if (!signedIn) {
        return (
            <section className="flex flex-col gap-2">
                <h2 className="flex items-center gap-1.5 text-sm font-semibold">
                    <FiUser className="size-3.5" />
                    Your avatar
                </h2>
                <p className="rounded-lg border border-dashed border-border px-4 py-5 text-sm text-muted">
                    Sign in to see and change the avatar you wear in game.
                </p>
            </section>
        )
    }

    // A site with avatars switched off answers 404; there is nothing to show.
    if (mine.error || schema.error) {
        const message = messageOf(mine.error ?? schema.error)

        if (/not.?found|not available/i.test(message)) return null

        return (
            <section className="text-sm text-muted">
                Your avatar could not be loaded: {message}
            </section>
        )
    }

    if (!mine.data || !schema.data) {
        return (
            <section className="h-56 animate-pulse rounded-xl border border-border bg-surface" />
        )
    }

    const doc = draft ?? mine.data.avatar
    const dirty = draft !== null

    return (
        <section className="flex flex-col gap-3">
            <h2 className="flex items-center gap-1.5 text-sm font-semibold">
                <FiUser className="size-3.5" />
                Your avatar
            </h2>

            <div
                role="tablist"
                aria-label="Avatar"
                className="-mx-3 flex gap-1.5 overflow-x-auto px-3"
            >
                {TABS.map(({ key, label, icon: Icon }) => (
                    <button
                        key={key}
                        type="button"
                        role="tab"
                        aria-selected={tab === key}
                        onClick={() => setTab(key)}
                        className={`flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-2 text-sm ${
                            tab === key
                                ? 'border-accent bg-accent text-accent-foreground'
                                : 'border-border hover:bg-surface-2'
                        }`}
                    >
                        <Icon className="hidden size-3.5 min-[420px]:block" />
                        {label}
                    </button>
                ))}
            </div>

            <div className="rounded-xl border border-border bg-surface p-3">
                {tab === 'avatar' ? (
                    <div className="flex flex-col gap-4 sm:flex-row">
                        <div className="flex flex-col items-center gap-2 sm:items-start">
                            <Figure doc={doc} twoD={mine.data.twoD?.url ?? null} />
                            {mine.data.model && (
                                <span className="text-xs text-muted">
                                    Wearing {mine.data.model.name}
                                </span>
                            )}
                            {!mine.data.saved && !dirty && (
                                <span className="text-xs text-muted">
                                    The default figure. Change something and save.
                                </span>
                            )}
                        </div>

                        <SlotEditor
                            schema={schema.data}
                            doc={doc}
                            onChange={setDraft}
                        />
                    </div>
                ) : tab === 'WEARABLES' ? (
                    <WearablesTab manageUrl={manageUrl} />
                ) : (
                    <ActionTab kind={tab} manageUrl={manageUrl} />
                )}
            </div>

            {tab === 'avatar' && (
                <div className="flex flex-wrap items-center gap-2">
                    <button
                        type="button"
                        disabled={!dirty || save.isPending}
                        onClick={() => draft && save.mutate(draft)}
                        className="flex items-center gap-1.5 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-accent-foreground disabled:opacity-50"
                    >
                        <FiSave className="size-3.5" />
                        {save.isPending ? 'Saving…' : 'Save avatar'}
                    </button>
                    <button
                        type="button"
                        disabled={!dirty || save.isPending}
                        onClick={() => setDraft(null)}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-2 text-sm disabled:opacity-50"
                    >
                        <FiRefreshCw className="size-3.5" />
                        Discard
                    </button>
                    {save.error && (
                        <span className="text-xs text-danger">
                            {messageOf(save.error)}
                        </span>
                    )}
                </div>
            )}
        </section>
    )
}

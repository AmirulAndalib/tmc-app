import { z } from 'zod'

/**
 * Friends, presence, parties and player statistics — the game backbone's
 * routes under `/api/app/v1`, as the app reads them.
 *
 * **Not part of `contract.ts`, and deliberately so.** That file is a verbatim
 * mirror of website-city's `src/types/app-api/contract.ts`, and these routes'
 * shapes are not in it: they are documented in website-city's
 * `docs/api/app-social.md` and typed next to their handlers
 * (`lib/user/social/presence.ts`, `lib/party/app-view.ts`,
 * `lib/integration/stats.ts`). Mirroring them here is the same bargain the
 * contract makes — a drift is a loud parse error on the first request — and the
 * day they move into the contract, this file becomes re-exports of it.
 *
 * Every schema is written from what a route SENDS, lenient only where the docs
 * say a field may be missing on an older server, so a genuinely new shape still
 * fails loudly rather than rendering `undefined`.
 *
 * All of them accept the device token the app holds. A device credential is
 * bound to no game, so nothing here is narrowed to one (docs: "A device
 * credential is bound to no app and is not narrowed").
 */

// ------------------------------------------------------------------ Presence

export const PresenceStatusVals = ['offline', 'online', 'in_game', 'away'] as const
export const PresenceStatusSchema = z.enum(PresenceStatusVals)
export type PresenceStatusT = z.infer<typeof PresenceStatusSchema>

/**
 * Where a friend is. Offline is blank: every nullable field null and
 * `joinable` false — which is also what a member hiding their activity reads
 * as, to everybody.
 */
export const PresenceSchema = z.object({
    status: PresenceStatusSchema,
    appId: z.number().int().nullable(),
    appName: z.string().nullable(),
    serverId: z.number().int().nullable(),
    serverName: z.string().nullable(),
    partyId: z.string().nullable(),
    joinable: z.boolean(),
    detail: z.string().nullable(),
    updatedAt: z.string().nullable(),
})
export type PresenceT = z.infer<typeof PresenceSchema>

// ------------------------------------------------------------------- Friends

export const FriendSchema = z.object({
    userId: z.string(),
    displayName: z.string(),
    avatarUrl: z.string().nullable(),
    presence: PresenceSchema,
})
export type FriendT = z.infer<typeof FriendSchema>

export const FriendListSchema = z.array(FriendSchema)

/**
 * A pending request. `id` is the friendship row's INTEGER id and goes back as
 * `requestId`; `userId` is always the OTHER person.
 */
export const FriendRequestSchema = z.object({
    id: z.number().int(),
    userId: z.string(),
    displayName: z.string(),
    createdAt: z.string(),
})
export type FriendRequestT = z.infer<typeof FriendRequestSchema>

export const FriendRequestsSchema = z.object({
    incoming: z.array(FriendRequestSchema),
    outgoing: z.array(FriendRequestSchema),
})
export type FriendRequestsT = z.infer<typeof FriendRequestsSchema>

export const FriendRequestSentSchema = z.object({ id: z.number().int() })

// ------------------------------------------------------------------- Parties

export const PartyMemberSchema = z.object({
    userId: z.string(),
    displayName: z.string().nullable(),
    gameName: z.string().nullable().default(null),
    role: z.string(),
    state: z.string(),
    presence: z.string().nullable().default(null),
    joinedAt: z.string().nullable().default(null),
    leftAt: z.string().nullable().default(null),
    readyAt: z.string().nullable().default(null),
    connectedAt: z.string().nullable().default(null),
})
export type PartyMemberT = z.infer<typeof PartyMemberSchema>

/**
 * One party, flat, with its stage and roster. Ids are decimal STRINGS — they
 * can pass 2^53 — and `hostId` is null when the host is somebody this viewer
 * may not name.
 */
export const PartySchema = z.object({
    id: z.string(),
    name: z.string().nullable(),
    type: z.string(),
    techType: z.string().nullable().default(null),
    maxUsers: z.number().int().nullable(),
    users: z.number().int(),
    stage: z.string(),
    startTime: z.string().nullable().default(null),
    endTime: z.string().nullable().default(null),
    mapName: z.string().nullable().default(null),
    gameMode: z.string().nullable().default(null),
    hostId: z.string().nullable(),
    appId: z.number().int().nullable(),
    serverId: z.number().int().nullable(),
    members: z.array(PartyMemberSchema).default([]),
})
export type PartyT = z.infer<typeof PartySchema>

export const PartyMineSchema = PartySchema.nullable()

export const PartyInviteSchema = z.object({
    id: z.string(),
    partyId: z.string(),
    partyName: z.string().nullable(),
    inviterId: z.string().nullable(),
    message: z.string().nullable().default(null),
})
export type PartyInviteT = z.infer<typeof PartyInviteSchema>

export const PartyInvitesSchema = z.array(PartyInviteSchema)

/** `leave`'s answer: whether the caller left, and whether that ended it. */
export const PartyLeaveSchema = z.object({ left: z.boolean(), ended: z.boolean() })

/** The routes that answer `data: null` on success. */
export const NothingSchema = z.unknown()

// --------------------------------------------------------------- Statistics

export const StatKindVals = ['COUNTER', 'HIGHEST', 'LOWEST', 'LATEST'] as const

/**
 * `kind` is kept a string rather than the enum above: a stat kind added on the
 * site is a stat the app can still RANK, since the ordering comes back
 * already applied. The list is for labels only.
 */
export const StatValueSchema = z.object({
    key: z.string(),
    name: z.string(),
    kind: z.string(),
    unit: z.string(),
    decimals: z.number().int().nonnegative(),
    value: z.number(),
})
export type StatValueT = z.infer<typeof StatValueSchema>

/** `GET stats/me?app=` — every value this member holds in one game. */
export const StatsMeSchema = z.object({
    player: z.string(),
    name: z.string().nullable(),
    stats: z.array(StatValueSchema),
})
export type StatsMeT = z.infer<typeof StatsMeSchema>

export const StatsTopRowSchema = z.object({
    rank: z.number().int().positive(),
    player: z.string(),
    name: z.string(),
    value: z.number(),
})
export type StatsTopRowT = z.infer<typeof StatsTopRowSchema>

/** `GET stats/top?app=&stat=` — one stat as a ranking, with the caller's own row. */
export const StatsTopSchema = z.object({
    stat: z.object({
        key: z.string(),
        name: z.string(),
        kind: z.string(),
        unit: z.string(),
        decimals: z.number().int().nonnegative(),
        players: z.number().int().nonnegative(),
    }),
    rows: z.array(StatsTopRowSchema),
    self: StatsTopRowSchema.nullable(),
})
export type StatsTopT = z.infer<typeof StatsTopSchema>

/** `GET stats/defs?app=` — every visible stat the game declared, by key. */
export const StatDefSchema = z.object({
    key: z.string(),
    name: z.string(),
    description: z.string().nullable(),
    kind: z.string(),
    unit: z.string(),
    decimals: z.number().int().nonnegative(),
    players: z.number().int().nonnegative(),
})
export type StatDefT = z.infer<typeof StatDefSchema>
export const StatDefsSchema = z.array(StatDefSchema)

/** website-city's `STATS_MAX_KEY`. */
export const STAT_KEY_MAX = 64

/**
 * A stat key, as the site's `IdentifierText` accepts it: the page's `?stat=`
 * is typed by a person, and a key the site would refuse is better caught
 * before the request than turned into a 400 toast.
 */
export function isStatKey(raw: string): boolean {
    return raw.length <= STAT_KEY_MAX && /^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(raw)
}

/**
 * A value in the stat's own terms: its decimals, then its unit.
 *
 * `decimals` is clamped because it comes from a game's own definition and
 * `toFixed` throws past 100.
 */
export function formatStat(value: number, decimals: number, unit: string): string {
    const places = Math.min(Math.max(decimals, 0), 6)
    const text = value.toLocaleString(undefined, {
        minimumFractionDigits: places,
        maximumFractionDigits: places,
    })

    return unit ? `${text} ${unit}` : text
}

/** How a presence reads in one short line. */
export function presenceLabel(p: PresenceT): string {
    switch (p.status) {
        case 'in_game': {
            const where = p.serverName ?? p.appName

            return where ? `Playing ${where}` : 'In a game'
        }
        case 'online':
            return 'Online'
        case 'away':
            return 'Away'
        default:
            return 'Offline'
    }
}

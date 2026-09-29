import {
    STAT_KEY_MAX,
    type AppFriendRequestT,
    type AppFriendT,
    type AppPartyT,
    type AppPresenceT,
    type StatsTopResponseT,
} from './contract'

/**
 * Friends, presence, parties and player statistics — the game backbone's
 * routes under `/api/app/v1` — as the UI reads them.
 *
 * **The schemas are in the contract** (`./contract`, section "Social", mirrored
 * from website-city by `npm run contract:sync`); `client.ts` parses through
 * them directly. What is left here is what the site has no reason to own: the
 * short names the routes' components use, and the display helpers.
 *
 * All of these routes accept the device token the app holds. A device
 * credential is bound to no game, so nothing here is narrowed to one.
 */

export type PresenceT = AppPresenceT
export type FriendT = AppFriendT
export type FriendRequestT = AppFriendRequestT
export type PartyT = AppPartyT
export type StatsTopT = StatsTopResponseT

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

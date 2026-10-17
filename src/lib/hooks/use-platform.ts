import { useState } from 'react'
import { type } from '@tauri-apps/plugin-os'

/**
 * Which OS this build is running on.
 *
 * Deliberately narrow in what it is FOR. Layout is keyed on the window (see
 * `use-breakpoint`) and must stay that way — a desktop window dragged narrow
 * wants the phone layout, which no platform check can tell you. This exists for
 * the handful of decisions that genuinely are about the platform: whether the
 * app draws its own window frame at all, and which side that frame's buttons go
 * on.
 *
 * `null` until the plugin answers, which is one paint. Callers treat `null` as
 * "not desktop" so nothing flashes a titlebar onto a phone.
 */
export type PlatformT = 'windows' | 'macos' | 'linux' | 'android' | 'ios' | null

const DESKTOP = new Set<PlatformT>(['windows', 'macos', 'linux'])

export function usePlatform(): PlatformT {
    // Read once, synchronously: `type()` needs no round trip, so there is no
    // first frame without it.
    const [platform] = useState<PlatformT>(() => {
        try {
            return type()
        } catch {
            // A browser-only `npm run dev` has no Tauri bridge. Staying null
            // means the web preview shows no window chrome, which is right —
            // the browser is already drawing some.
            return null
        }
    })

    return platform
}

/** True only once the platform is known to be a desktop one. */
export function useIsDesktop(): boolean {
    return DESKTOP.has(usePlatform())
}

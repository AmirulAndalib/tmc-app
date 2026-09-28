import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { FiAlertTriangle, FiCheck, FiDownload, FiRefreshCw } from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import type { UpdaterOriginT, UpdateCheckT } from '~/lib/ipc/schemas'
import { useSettings } from '~/lib/settings/provider'
import { Row, Select } from '~/components/form'

/**
 * Settings → App → Updates: the channel, a "check now" button, and — behind a
 * disclosure — where updates come from and which key they must be signed with.
 *
 * The last two are the ADMINISTRATOR half and are shown, never edited: the key
 * decides what the app will install over itself, so no command sets it. They
 * come from a hand-edited `settings.json` (validated on read, audited at
 * Security level at startup) or the build environment.
 *
 * With no key anywhere the app still checks and still links to the download
 * page; it just says it cannot install, and why.
 */

const ORIGIN_LABEL: Record<UpdaterOriginT, string> = {
    override: 'settings.json',
    compiled: 'built in',
    site: 'the site',
}

const CHANNELS = [
    { value: 'stable', label: 'Stable' },
    { value: 'beta', label: 'Beta (pre-releases)' },
]

export default function UpdaterSettings() {
    const { setApp } = useSettings()
    const client = useQueryClient()
    const [found, setFound] = useState<UpdateCheckT | null>(null)
    const [installed, setInstalled] = useState(false)

    const status = useQuery({
        queryKey: ['updater-status'],
        queryFn: ipc.updaterStatus,
    })

    const check = useMutation({
        mutationFn: ipc.updateCheck,
        onSuccess: (result) => {
            setFound(result)
            setInstalled(false)
        },
    })

    const install = useMutation({
        mutationFn: ipc.updateInstall,
        onSuccess: (result) => setInstalled(result.outdated),
    })

    const s = status.data

    if (status.error)
        return (
            <Row
                label="Updater"
                control={
                    <span className="text-xs text-danger">
                        {messageOf(status.error)}
                    </span>
                }
            />
        )

    if (!s) return null

    return (
        <>
            {s.supported && (
                <Row
                    label="Channel"
                    hint="Beta gets pre-releases as well as every stable release."
                    control={
                        <Select
                            label="Update channel"
                            value={s.channel}
                            options={CHANNELS}
                            onChange={(channel) => {
                                void setApp({
                                    updateChannel: channel as 'stable' | 'beta',
                                }).then(() =>
                                    client.invalidateQueries({
                                        queryKey: ['updater-status'],
                                    })
                                )
                            }}
                        />
                    }
                />
            )}

            <Row
                label="Installs updates itself"
                hint={
                    !s.supported
                        ? 'Not on this platform — updates come from the store or the download page.'
                        : s.available
                          ? `Signed with the ${s.keyOrigin ? ORIGIN_LABEL[s.keyOrigin] : ''} key, from ${ORIGIN_LABEL[s.endpointOrigin]}.`
                          : 'No update signing key is configured, so this build can only point you at the download page.'
                }
                control={
                    s.available ? (
                        <FiCheck className="size-4 text-success" aria-label="Yes" />
                    ) : (
                        <FiAlertTriangle
                            className="size-4 text-warning"
                            aria-label="No"
                        />
                    )
                }
            />

            <div className="flex flex-col gap-2 px-3 py-2.5">
                <div className="flex flex-wrap items-center gap-2">
                    <button
                        type="button"
                        disabled={check.isPending}
                        onClick={() => check.mutate()}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-60"
                    >
                        <FiRefreshCw
                            className={`size-3 ${check.isPending ? 'animate-spin' : ''}`}
                        />
                        Check for updates
                    </button>

                    {found?.outdated && found.installable && !installed && (
                        <button
                            type="button"
                            disabled={install.isPending}
                            onClick={() => install.mutate()}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground disabled:opacity-60"
                        >
                            <FiDownload className="size-3" />
                            {install.isPending
                                ? 'Installing…'
                                : `Install ${found.latest ?? ''}`}
                        </button>
                    )}
                </div>

                <p className="text-xs text-muted" aria-live="polite">
                    {check.error
                        ? messageOf(check.error)
                        : install.error
                          ? messageOf(install.error)
                          : installed
                            ? 'Installed. Restart the app to finish.'
                            : found
                              ? found.outdated
                                  ? `Version ${found.latest} is available (you have ${found.current}).`
                                  : `You are up to date (${found.current}).`
                              : null}
                </p>
            </div>

            {s.supported && <SourceForm />}
        </>
    )
}

/**
 * The endpoint and key, READ-ONLY. Nothing in the app can change them: the key
 * decides what installs over this program, and the webview is assumed hostile.
 * An administrator sets them in `settings.json` with the app closed.
 */
function SourceForm() {
    const status = useQuery({
        queryKey: ['updater-status'],
        queryFn: ipc.updaterStatus,
    })
    const s = status.data

    if (!s) return null

    const overridden = s.endpointOverride !== null || s.pubkeyOverride !== null

    return (
        <details className="px-3 py-2.5" open={overridden}>
            <summary className="cursor-pointer text-sm">
                Update source (administrators)
                {overridden && (
                    <span className="ml-2 text-xs text-warning">overridden</span>
                )}
            </summary>

            <div className="mt-3 flex flex-col gap-3">
                <p className="text-xs text-muted">
                    Updates are fetched from{' '}
                    <span className="break-all font-mono">{s.endpoint}</span> (
                    {ORIGIN_LABEL[s.endpointOrigin]})
                    {s.keyOrigin === null
                        ? ' and cannot be installed: no signing key is configured.'
                        : ` and must be signed with the ${ORIGIN_LABEL[s.keyOrigin]} key.`}
                </p>

                {s.pubkeyOverride !== null && (
                    <div className="flex flex-col gap-1 text-xs">
                        <span className="font-medium">Public key</span>
                        <code className="break-all rounded-lg border border-border px-2 py-1.5 text-[11px]">
                            {s.pubkeyOverride}
                        </code>
                    </div>
                )}

                <p className="flex items-start gap-1.5 text-xs text-muted">
                    <FiAlertTriangle className="mt-0.5 size-3 shrink-0" />
                    <span>
                        These cannot be changed from inside the app. An
                        administrator sets <code>updaterEndpoint</code> (an HTTPS
                        manifest URL) and <code>updaterPubkey</code> (the key{' '}
                        <code>tauri signer generate</code> printed) in the
                        app&apos;s <code>settings.json</code> while the app is
                        closed; a value that fails validation is ignored, and an
                        override in force is written to the security log at every
                        start. Removing them returns to the{' '}
                        {s.compiledKey ? 'built-in' : 'default'} source.
                    </span>
                </p>
            </div>
        </details>
    )
}

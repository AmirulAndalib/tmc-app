import { useEffect, useState } from 'react'
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
 * The last two are the ADMINISTRATOR half. The key decides what the app will
 * install over itself, so it is its own command (`updater_set_source`, which
 * validates and audits at Security level), never part of a settings patch, and
 * this screen says so beside the Save button rather than in a doc nobody opens.
 *
 * With no key anywhere the app still checks and still links to the download
 * page; it just says it cannot install, and why.
 */

const ORIGIN_LABEL: Record<UpdaterOriginT, string> = {
    override: 'set here',
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

/** The endpoint and key overrides. Collapsed: most people never open it. */
function SourceForm() {
    const client = useQueryClient()
    const status = useQuery({
        queryKey: ['updater-status'],
        queryFn: ipc.updaterStatus,
    })
    const s = status.data

    const [endpoint, setEndpoint] = useState('')
    const [pubkey, setPubkey] = useState('')

    useEffect(() => {
        setEndpoint(s?.endpointOverride ?? '')
        setPubkey(s?.pubkeyOverride ?? '')
    }, [s?.endpointOverride, s?.pubkeyOverride])

    const save = useMutation({
        mutationFn: (next: { endpoint: string | null; pubkey: string | null }) =>
            ipc.updaterSetSource(next.endpoint, next.pubkey),
        onSuccess: (next) => client.setQueryData(['updater-status'], next),
    })

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
                    <span className="break-all font-mono">{s.endpoint}</span>
                    {s.keyOrigin === null
                        ? ' and cannot be installed: no signing key is configured.'
                        : ` and must be signed with the ${ORIGIN_LABEL[s.keyOrigin]} key.`}
                </p>

                <label className="flex flex-col gap-1 text-xs">
                    <span className="font-medium">Endpoint</span>
                    <input
                        value={endpoint}
                        onChange={(e) => setEndpoint(e.target.value)}
                        placeholder={s.compiledEndpoint}
                        spellCheck={false}
                        className="rounded-lg border border-border bg-transparent px-2 py-1.5 font-mono text-xs"
                    />
                    <span className="text-muted">
                        HTTPS. A <code>latest.json</code> manifest, or a route that
                        takes <code>{'{{target}}'}</code>, <code>{'{{arch}}'}</code>{' '}
                        and <code>{'{{current_version}}'}</code>. Empty uses the
                        built-in one.
                    </span>
                </label>

                <label className="flex flex-col gap-1 text-xs">
                    <span className="font-medium">Public key</span>
                    <textarea
                        value={pubkey}
                        onChange={(e) => setPubkey(e.target.value)}
                        placeholder={
                            s.compiledKey
                                ? 'Empty uses the key built into this app.'
                                : 'No key is built into this app. Paste the one `tauri signer generate` printed.'
                        }
                        rows={3}
                        spellCheck={false}
                        className="rounded-lg border border-border bg-transparent px-2 py-1.5 font-mono text-[11px]"
                    />
                </label>

                <p className="flex items-start gap-1.5 text-xs text-warning">
                    <FiAlertTriangle className="mt-0.5 size-3 shrink-0" />
                    The key decides what this app will install over itself. Only
                    change it to a key you were given by whoever builds your
                    releases. Every change is written to the security log.
                </p>

                <div className="flex flex-wrap items-center gap-2">
                    <button
                        type="button"
                        disabled={save.isPending}
                        onClick={() =>
                            save.mutate({
                                endpoint: endpoint.trim() || null,
                                pubkey: pubkey.trim() || null,
                            })
                        }
                        className="rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground disabled:opacity-60"
                    >
                        Save
                    </button>
                    {overridden && (
                        <button
                            type="button"
                            disabled={save.isPending}
                            onClick={() =>
                                save.mutate({ endpoint: null, pubkey: null })
                            }
                            className="rounded-lg border border-border px-3 py-1.5 text-xs"
                        >
                            Use the built-in source
                        </button>
                    )}
                    {save.error && (
                        <span className="text-xs text-danger">
                            {messageOf(save.error)}
                        </span>
                    )}
                    {save.isSuccess && !save.isPending && (
                        <span className="text-xs text-success">Saved.</span>
                    )}
                </div>
            </div>
        </details>
    )
}

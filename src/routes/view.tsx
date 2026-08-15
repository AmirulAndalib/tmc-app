import { useEffect, useRef } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { openUrl } from '@tauri-apps/plugin-opener'
import {
    FiArrowLeft,
    FiDownload,
    FiExternalLink,
    FiGitBranch,
    FiPlay,
} from 'react-icons/fi'
import { Button } from '@modcommunity/shared'

import { api } from '~/lib/api/client'
import {
    ContentKindSchema,
    type ContentKindT,
    type ContentSummaryT,
} from '~/lib/api/contract'
import { appLabel } from '~/lib/api/labels'
import { opensExternally } from '~/lib/external'
import Markdown from '~/components/markdown'
import ServerPanel from '~/components/server-panel'
import InstallButton from '~/components/install-button'
import SubscribeButton from '~/components/subscribe-button'
import Dependencies from '~/components/dependencies'
import Gallery from '~/components/gallery'
import ReportButton from '~/components/report-button'
import Reviews from '~/components/reviews'

/**
 * A content item's page — assets, mods, servers, maps, articles, communities,
 * collections and members, all through one route.
 *
 * Same reasoning as the card: the API returns one `ContentDetail` shape, so
 * this is one component with a few kind-specific blocks rather than eight
 * near-identical pages that drift.
 */
export default function ViewRoute() {
    const params = useParams<{ kind: string; id: string }>()
    const navigate = useNavigate()
    const [search] = useSearchParams()

    /*
     * Set by a `tmc://install/…` deep link — see `lib/hooks/use-deep-link`. All
     * it does is draw a ring around the install controls, because somebody who
     * pressed a button in their browser and landed here needs to be shown where
     * the next button is. It does not install anything: a link that could would
     * be a remote install primitive reachable from any web page.
     */
    const highlight = search.get('install') === '1'

    const parsed = ContentKindSchema.safeParse(params.kind)
    const kind = parsed.success ? parsed.data : null
    const id = params.id ?? ''

    const detail = useQuery({
        queryKey: ['content', kind, id],
        queryFn: () => api.content(kind!, id),
        enabled: Boolean(kind && id),
        staleTime: 60 * 1000,
    })

    if (!kind)
        return <Centered>That is not a content type this app knows about.</Centered>

    if (detail.isPending) return <Centered>Loading…</Centered>

    if (detail.isError) return <Centered>{detail.error.message}</Centered>

    const { summary, content, rules, releases, media, links, dependencies } =
        detail.data
    const banner = summary.images.banner ?? summary.images.card

    /*
     * An article's BODY belongs to the website — see `lib/external` for why.
     *
     * The rest of this page still applies: the banner, the author, the tags and
     * the stats are all things the app renders well, so only the markdown
     * section is withheld and replaced with the way out.
     */
    const external = opensExternally(summary.kind)

    const latest = releases[0]
    const download = latest?.files[0]

    return (
        <article className="flex flex-col">
            <div className="relative">
                {banner ? (
                    <img
                        src={banner}
                        alt=""
                        className="h-40 w-full object-cover sm:h-56"
                    />
                ) : (
                    <div className="h-24 w-full bg-surface-secondary" />
                )}

                <button
                    type="button"
                    onClick={() => navigate(-1)}
                    aria-label="Back"
                    className="absolute left-3 top-3 rounded-full bg-background/80 p-2 backdrop-blur"
                    style={{ top: 'calc(0.75rem + var(--safe-top))' }}
                >
                    <FiArrowLeft className="size-4" />
                </button>
            </div>

            <div className="flex flex-col gap-4 p-4">
                <header className="flex flex-col gap-2">
                    <div className="flex flex-wrap items-center gap-2">
                        {summary.app && (
                            <span className="text-xs font-medium text-accent">
                                {appLabel(summary.app)}
                            </span>
                        )}
                        {summary.isOfficial && (
                            <span className="rounded-full bg-accent px-2 py-0.5 text-[0.65rem] text-accent-foreground">
                                Official
                            </span>
                        )}
                        {summary.archived && (
                            <span className="rounded-full bg-surface-tertiary px-2 py-0.5 text-[0.65rem]">
                                No longer maintained
                            </span>
                        )}
                    </div>

                    <h1 className="selectable text-2xl font-bold">
                        {summary.name}
                    </h1>

                    {summary.description && (
                        <p className="selectable text-sm text-muted">
                            {summary.description}
                        </p>
                    )}

                    {summary.owner && (
                        <div className="flex items-center gap-2 text-xs text-muted">
                            {summary.owner.avatar && (
                                <img
                                    src={summary.owner.avatar}
                                    alt=""
                                    className="size-5 rounded-full object-cover"
                                />
                            )}
                            by{' '}
                            {summary.owner.username ??
                                summary.owner.name ??
                                'unknown'}
                        </div>
                    )}
                </header>

                {/* ------------------------------------------------ Actions */}
                {highlight && (
                    <p className="rounded-lg border border-accent/50 bg-accent/10 px-3 py-2 text-xs">
                        You asked to install this from the website. Choose how below
                        — subscribing keeps it updated and follows you to your other
                        devices.
                    </p>
                )}

                <div
                    className={`flex flex-wrap gap-2 ${
                        highlight
                            ? 'rounded-xl ring-2 ring-accent ring-offset-2 ring-offset-background'
                            : ''
                    }`}
                >
                    {summary.kind === 'server' && summary.server?.connectUrl && (
                        <Button
                            btnType="primary"
                            onClick={() =>
                                void openUrl(summary.server!.connectUrl!)
                            }
                        >
                            <span className="flex items-center gap-2">
                                <FiPlay className="size-4" /> Join server
                            </span>
                        </Button>
                    )}

                    {/*
                        Subscribe first: it is what the app is FOR, and it is
                        the one that keeps working after the release this page
                        is showing has been superseded. The plugin-bundle
                        installer stays beside it for the games covered by a
                        user-installed plugin rather than a shipped rule.
                    */}
                    {(summary.kind === 'mod' ||
                        summary.kind === 'asset' ||
                        summary.kind === 'collection') && (
                        <SubscribeButton summary={summary} />
                    )}

                    {(summary.kind === 'mod' || summary.kind === 'asset') && (
                        <InstallButton summary={summary} releases={releases} />
                    )}

                    {download && (
                        <Button
                            btnType="secondary"
                            onClick={() => void openUrl(download.url)}
                        >
                            <span className="flex items-center gap-2">
                                <FiDownload className="size-4" />
                                Download
                                {latest?.version ? ` ${latest.version}` : ''}
                            </span>
                        </Button>
                    )}

                    <Button
                        btnType={external ? 'primary' : 'secondary'}
                        onClick={() => void openUrl(summary.webUrl)}
                    >
                        <span className="flex items-center gap-2">
                            <FiExternalLink className="size-4" />
                            {external ? 'Read the article' : 'Open on the web'}
                        </span>
                    </Button>
                </div>

                {/* --------------------------------------------- Live server */}
                {summary.kind === 'server' && <ServerPanel item={summary} />}

                {/* ------------------------------------------------- Chips */}
                {(summary.categories.length > 0 || summary.tags.length > 0) && (
                    <div className="flex flex-wrap gap-1.5">
                        {summary.categories.map((cat) => (
                            <span
                                key={`c-${cat.id}`}
                                className="rounded-full bg-surface-secondary px-2 py-0.5 text-xs text-muted"
                            >
                                {cat.name}
                            </span>
                        ))}
                        {summary.tags.map((tag) => (
                            <span
                                key={`t-${tag.id}`}
                                className="rounded-full border border-border px-2 py-0.5 text-xs text-muted"
                            >
                                #{tag.name}
                            </span>
                        ))}
                    </div>
                )}

                {/* ------------------------------------------------- Stats */}
                <Stats summary={summary} />

                {/* --------------------------------------------- Screenshots */}
                <Gallery
                    items={media
                        .filter((m) => m.url)
                        .map((m) => ({
                            id: m.id,
                            url: m.url!,
                            title: m.title,
                        }))}
                />

                {external ? (
                    <ExternalBody summary={summary} />
                ) : (
                    content && (
                        <section className="selectable">
                            <Markdown source={content} />
                        </section>
                    )
                )}

                {rules && (
                    <section className="selectable rounded-xl border border-border bg-surface p-3">
                        <h2 className="mb-2 text-sm font-semibold">Rules</h2>
                        <Markdown source={rules} />
                    </section>
                )}

                {releases.length > 0 && (
                    <section>
                        <h2 className="mb-2 text-sm font-semibold">Releases</h2>
                        <ul className="flex flex-col gap-1.5">
                            {releases.map((release) => (
                                <li
                                    key={release.id}
                                    className="flex items-center justify-between rounded-lg border border-border bg-surface px-3 py-2 text-sm"
                                >
                                    <span>
                                        {release.version ?? `#${release.id}`}
                                        <span className="ml-2 text-xs text-muted">
                                            {new Date(
                                                release.createdAt
                                            ).toLocaleDateString()}
                                        </span>
                                    </span>

                                    {release.files[0] && (
                                        <button
                                            type="button"
                                            onClick={() =>
                                                void openUrl(release.files[0]!.url)
                                            }
                                            className="text-accent"
                                            aria-label="Download this release"
                                        >
                                            <FiDownload className="size-4" />
                                        </button>
                                    )}
                                </li>
                            ))}
                        </ul>
                    </section>
                )}

                <Dependencies items={dependencies} />

                {/*
                 * Reviews last, under everything the item itself says. They are
                 * the longest section by far and putting them above the release
                 * list would bury the thing most people came for.
                 */}
                {REVIEWABLE.has(summary.kind) && (
                    <Reviews kind={summary.kind} id={Number(summary.id)} />
                )}

                <div className="flex justify-end">
                    <ReportButton
                        kind={summary.kind}
                        id={summary.id}
                        name={summary.name}
                    />
                </div>

                {links.length > 0 && (
                    <section>
                        <h2 className="mb-2 text-sm font-semibold">Links</h2>
                        <div className="flex flex-wrap gap-2">
                            {links.map((link) => (
                                <button
                                    key={link.id}
                                    type="button"
                                    onClick={() => void openUrl(link.url)}
                                    className="flex items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs text-muted hover:text-foreground"
                                >
                                    {link.isSource ? (
                                        <FiGitBranch className="size-3" />
                                    ) : (
                                        <FiExternalLink className="size-3" />
                                    )}
                                    {link.name ?? new URL(link.url).hostname}
                                </button>
                            ))}
                        </div>
                    </section>
                )}
            </div>
        </article>
    )
}

/**
 * Kinds that have reviews.
 *
 * Mirrors `REVIEW_FIELD` on the server, which is the authority: a `collection`
 * is a list somebody made and the things in it carry the opinions, and a
 * `user` is reviewed on the website through a different surface entirely.
 * Asking for reviews of a kind the server has no column for would be a 400 on
 * every one of those pages.
 */
const REVIEWABLE = new Set<ContentKindT>([
    'mod',
    'asset',
    'server',
    'serverMap',
    'article',
    'community',
])

/**
 * The numbers under the title.
 *
 * The website shows these in a sidebar; an app window is too narrow for one, so
 * they are a row. Zeroes are omitted rather than shown — a fresh mod reading
 * "0 downloads · 0 favourites · 0 comments" is three facts nobody needed and
 * one impression ("nobody uses this") that the numbers do not support.
 */
function Stats({ summary }: { summary: ContentSummaryT }) {
    const entries: { label: string; value: string }[] = []

    const add = (value: number, one: string, many: string) => {
        if (value > 0)
            entries.push({
                label: value === 1 ? one : many,
                value: value.toLocaleString(),
            })
    }

    add(summary.stats.downloads, 'download', 'downloads')
    add(summary.stats.views, 'view', 'views')
    add(summary.stats.favorites, 'favourite', 'favourites')
    add(summary.stats.comments, 'comment', 'comments')

    if (summary.stats.rating !== null)
        entries.push({
            label: `from ${summary.stats.reviews} review${summary.stats.reviews === 1 ? '' : 's'}`,
            value: `${summary.stats.rating.toFixed(1)}★`,
        })

    if (entries.length === 0) return null

    return (
        <dl className="flex flex-wrap gap-x-5 gap-y-1 text-xs">
            {entries.map((entry) => (
                <div key={entry.label} className="flex items-baseline gap-1.5">
                    <dt className="sr-only">{entry.label}</dt>
                    <dd className="font-medium">{entry.value}</dd>
                    <span aria-hidden className="text-muted">
                        {entry.label}
                    </span>
                </div>
            ))}
        </dl>
    )
}

/**
 * What stands in for an article's body.
 *
 * The browser is opened ONCE per article, on arrival, because reaching this
 * route means the user asked to read the thing. It is guarded on the id rather
 * than on mount so that going back and forward between two articles opens each
 * of them, and re-rendering opens neither — an effect that fired on every paint
 * would spawn a browser tab per render.
 *
 * The page stays behind it rather than navigating away: closing the browser
 * should leave the app where the user left it, with a button to open it again
 * if the launch was swallowed by a window manager.
 */
function ExternalBody({ summary }: { summary: { id: string; webUrl: string } }) {
    const openedFor = useRef<string | null>(null)

    useEffect(() => {
        if (openedFor.current === summary.id) return

        openedFor.current = summary.id

        void openUrl(summary.webUrl)
    }, [summary.id, summary.webUrl])

    return (
        <section className="flex flex-col items-center gap-2 rounded-xl border border-border bg-surface p-6 text-center">
            <FiExternalLink className="size-5 text-muted" />
            <p className="text-sm font-medium">Opened in your browser</p>
            <p className="max-w-sm text-xs text-muted">
                Articles are laid out for the website — media, callouts and all — so
                the app hands them to your browser rather than flattening them.
            </p>
            <button
                type="button"
                onClick={() => void openUrl(summary.webUrl)}
                className="mt-1 text-xs text-accent hover:underline"
            >
                Open it again
            </button>
        </section>
    )
}

function Centered({ children }: { children: React.ReactNode }) {
    return (
        <div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted">
            {children}
        </div>
    )
}

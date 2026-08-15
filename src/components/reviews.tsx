import { useState } from 'react'
import { useInfiniteQuery } from '@tanstack/react-query'
import { FiStar } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import type { ContentKindT } from '~/lib/api/contract'
import Markdown from '~/components/markdown'
import Select from '~/components/select'

/**
 * What people said about an item.
 *
 * READ ONLY, and the screen says so rather than showing a disabled "write a
 * review" button. A button that cannot be pressed is worse than no button: it
 * reads as broken, while a line of text reads as a decision.
 *
 * TWO THINGS THIS DOES THAT A LIST WOULD NOT
 * ------------------------------------------
 *   * **The distribution, as bars.** An average of 4.1 made of forty fives and
 *     ten ones is a different item from one made of fifty fours, and only the
 *     bars tell them apart. The server computes the counts, because doing it
 *     here would mean fetching every review to draw five bars.
 *   * **Review bodies through the markdown renderer.** They are member-authored
 *     text, so they go through the same sanitising renderer a mod description
 *     does — never `dangerouslySetInnerHTML`, and never raw.
 */

const SORTS = [
    { value: 'recent', label: 'Most recent' },
    { value: 'helpful', label: 'Most helpful' },
    { value: 'rating', label: 'Highest rated' },
] as const

type Sort = (typeof SORTS)[number]['value']

export default function Reviews({ kind, id }: { kind: ContentKindT; id: number }) {
    const [sort, setSort] = useState<Sort>('recent')

    const reviews = useInfiniteQuery({
        queryKey: ['reviews', kind, id, sort],
        queryFn: ({ pageParam }) =>
            api.reviews(kind, id, { sort, cursor: pageParam }),
        initialPageParam: null as string | null,
        getNextPageParam: (last) => last.nextCursor,
        staleTime: 60 * 1000,
    })

    const first = reviews.data?.pages[0]
    const rows = reviews.data?.pages.flatMap((page) => page.reviews) ?? []

    if (reviews.isPending)
        return (
            <section>
                <h2 className="mb-2 text-sm font-semibold">Reviews</h2>
                <p className="text-xs text-muted">Loading…</p>
            </section>
        )

    if (reviews.isError)
        return (
            <section>
                <h2 className="mb-2 text-sm font-semibold">Reviews</h2>
                <p className="text-xs text-muted">Reviews could not be loaded.</p>
            </section>
        )

    if (!first || first.total === 0)
        return (
            <section>
                <h2 className="mb-2 text-sm font-semibold">Reviews</h2>
                <p className="text-xs text-muted">Nobody has reviewed this yet.</p>
            </section>
        )

    const breakdown = [
        { score: 5, count: first.breakdown.five },
        { score: 4, count: first.breakdown.four },
        { score: 3, count: first.breakdown.three },
        { score: 2, count: first.breakdown.two },
        { score: 1, count: first.breakdown.one },
    ]

    const scored = breakdown.reduce((sum, row) => sum + row.count, 0)

    return (
        <section className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <h2 className="text-sm font-semibold">
                    Reviews
                    <span className="ml-2 text-xs font-normal text-muted">
                        {first.total}
                    </span>
                </h2>

                <Select
                    label="Sort reviews"
                    value={sort}
                    onChange={setSort}
                    options={SORTS.map((s) => ({
                        value: s.value,
                        label: s.label,
                    }))}
                />
            </div>

            {first.average !== null && (
                <div className="flex flex-col gap-2 rounded-xl border border-border bg-surface p-3 sm:flex-row sm:items-center sm:gap-6">
                    <div className="shrink-0 text-center">
                        <p className="text-2xl font-bold">
                            {first.average.toFixed(1)}
                        </p>
                        <Stars value={first.average} />
                        <p className="mt-1 text-[0.7rem] text-muted">
                            {scored} rating{scored === 1 ? '' : 's'}
                        </p>
                    </div>

                    <div className="flex min-w-0 flex-1 flex-col gap-1">
                        {breakdown.map((row) => (
                            <div
                                key={row.score}
                                className="flex items-center gap-2 text-[0.7rem]"
                            >
                                <span className="w-3 text-right text-muted">
                                    {row.score}
                                </span>
                                <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-surface-tertiary">
                                    <div
                                        className="h-full rounded-full bg-accent"
                                        style={{
                                            width: `${
                                                scored === 0
                                                    ? 0
                                                    : (row.count / scored) * 100
                                            }%`,
                                        }}
                                    />
                                </div>
                                <span className="w-8 text-right text-muted">
                                    {row.count}
                                </span>
                            </div>
                        ))}
                    </div>
                </div>
            )}

            <ul className="flex flex-col gap-2">
                {rows.map((review) => (
                    <li
                        key={review.id}
                        className="rounded-xl border border-border bg-surface p-3"
                    >
                        <div className="flex items-center gap-2">
                            {review.owner?.avatar ? (
                                <img
                                    src={review.owner.avatar}
                                    alt=""
                                    loading="lazy"
                                    className="size-6 rounded-full object-cover"
                                />
                            ) : (
                                <div className="size-6 rounded-full bg-surface-tertiary" />
                            )}

                            <span className="min-w-0 truncate text-sm">
                                {review.owner?.username ??
                                    review.owner?.name ??
                                    'Someone'}
                            </span>

                            {review.rating !== null && (
                                <Stars value={review.rating} />
                            )}

                            <span className="ml-auto shrink-0 text-[0.7rem] text-muted">
                                {new Date(review.createdAt).toLocaleDateString()}
                            </span>
                        </div>

                        {review.content && (
                            <div className="selectable mt-2 text-sm">
                                {/* Member-authored, so it goes through the same
                                    sanitising renderer a mod description does. */}
                                <Markdown source={review.content} />
                            </div>
                        )}
                    </li>
                ))}
            </ul>

            {reviews.hasNextPage && (
                <button
                    type="button"
                    disabled={reviews.isFetchingNextPage}
                    onClick={() => void reviews.fetchNextPage()}
                    className="self-center rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-50"
                >
                    {reviews.isFetchingNextPage ? 'Loading…' : 'Show more'}
                </button>
            )}
        </section>
    )
}

/**
 * Five stars, filled to `value`.
 *
 * The fill is a clipped overlay rather than half-star glyphs: a 4.3 is drawn as
 * 4.3, and there is no rounding decision to get wrong or to disagree with the
 * number printed beside it.
 */
function Stars({ value }: { value: number }) {
    const percent = Math.max(0, Math.min(100, (value / 5) * 100))

    return (
        <span
            className="relative inline-flex shrink-0"
            role="img"
            aria-label={`${value.toFixed(1)} out of 5`}
        >
            <span className="flex text-muted/40">
                {[0, 1, 2, 3, 4].map((n) => (
                    <FiStar key={n} className="size-3.5" aria-hidden />
                ))}
            </span>

            <span
                className="absolute inset-0 flex overflow-hidden text-warning"
                style={{ width: `${percent}%` }}
                aria-hidden
            >
                {[0, 1, 2, 3, 4].map((n) => (
                    <FiStar
                        key={n}
                        className="size-3.5 shrink-0"
                        fill="currentColor"
                    />
                ))}
            </span>
        </span>
    )
}

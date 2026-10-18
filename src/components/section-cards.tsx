import { Link } from 'react-router-dom'
import { FiArrowRight } from 'react-icons/fi'

import { useDownloads } from '~/lib/downloads/provider'
import { leaves, type NavNode } from '~/lib/nav'

/**
 * A card per child of a navigation group.
 *
 * The bottom bar cannot nest, so on a phone the landing page is the only way
 * to reach a child. Built from the same tree as the rail for that reason: a
 * section added to the rail appears here without anybody remembering to.
 */
export default function SectionCards({
    node,
    extra = [],
}: {
    node: NavNode
    /** Cards for places the rail deliberately does not list. */
    extra?: NavNode[]
}) {
    const downloads = useDownloads()

    return (
        <ul className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {[...leaves(node), ...extra].map((item) => (
                <li key={item.id}>
                    <Link
                        to={item.to}
                        className="group flex h-full items-start gap-3 rounded-xl border border-border bg-surface p-4 transition hover:border-accent hover:bg-surface-hover"
                    >
                        <span className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-accent/15 text-accent">
                            <item.icon className="size-5" />
                        </span>
                        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                            <span className="flex items-center gap-2 text-sm font-semibold">
                                {item.label}
                                {item.badge === 'downloads' &&
                                    downloads.active > 0 && (
                                        <span className="rounded-full bg-accent px-1.5 py-0.5 text-[0.6rem] font-medium text-accent-foreground">
                                            {downloads.active}
                                        </span>
                                    )}
                            </span>
                            <span className="text-xs text-muted">
                                {item.description}
                            </span>
                        </span>
                        <FiArrowRight className="mt-1 size-4 shrink-0 text-muted transition group-hover:translate-x-0.5 group-hover:text-accent" />
                    </Link>
                </li>
            ))}
        </ul>
    )
}

import { FiGlobe } from 'react-icons/fi'

/**
 * The bundled flags, so a flag draws offline and on Windows, where flag emoji
 * render as two letters.
 */
const FLAGS = import.meta.glob<string>(
    '/node_modules/country-flag-icons/3x2/*.svg',
    {
        eager: true,
        query: '?url',
        import: 'default',
    }
)

export function CountryFlag({
    code,
    className = 'h-3.5 w-5',
}: {
    code: string | null | undefined
    className?: string
}) {
    const src = code
        ? FLAGS[`/node_modules/country-flag-icons/3x2/${code.toUpperCase()}.svg`]
        : undefined

    if (!src) return <FiGlobe aria-hidden className="size-4 shrink-0 text-muted" />

    return (
        <img
            src={src}
            alt=""
            aria-hidden
            loading="lazy"
            className={`${className} shrink-0 rounded-[2px] object-cover shadow-[0_0_0_1px_rgb(0_0_0/0.15)]`}
        />
    )
}

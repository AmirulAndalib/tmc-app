import { useMutation, useQueryClient } from '@tanstack/react-query'
import { FiCheck, FiUserPlus } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'

/**
 * "Add friend" on a member's page.
 *
 * One POST, and the site decides everything about it: asking somebody who
 * already asked you ACCEPTS them, a decline makes only the declined sender wait
 * seven days, and a member with requests turned off answers
 * `friends.request.deny.closed`. The refusal's own sentence is shown as it
 * came, because it is already written for a person.
 *
 * Hidden when signed out — there is no account to be friends from — and on
 * the member's own page, which the site would refuse anyway.
 */
export default function AddFriendButton({ userId }: { userId: string }) {
    const { status, user } = useAuth()
    const client = useQueryClient()

    const send = useMutation({
        mutationFn: () => api.friendRequest(userId),
        onSettled: () => void client.invalidateQueries({ queryKey: ['social'] }),
    })

    if (status !== 'signedIn' || user?.id === userId) return null

    return (
        <div className="flex flex-col items-start gap-1">
            <button
                type="button"
                disabled={send.isPending || send.isSuccess}
                onClick={() => send.mutate()}
                className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-70"
            >
                {send.isSuccess ? (
                    <FiCheck className="size-3.5 text-success" />
                ) : (
                    <FiUserPlus className="size-3.5" />
                )}
                {send.isSuccess ? 'Request sent' : 'Add friend'}
            </button>
            {send.error && (
                <p className="text-[11px] text-danger">{messageOf(send.error)}</p>
            )}
        </div>
    )
}

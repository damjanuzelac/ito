import { create } from 'zustand'
import type { AuthUser } from '../../lib/main/store'
import { STORE_KEYS } from '../../lib/constants/store-keys'

// Single-user local build: the app always runs as the fixed self-hosted user.
const SELF_HOSTED_USER: AuthUser = {
  id: 'self-hosted',
  provider: 'self-hosted',
}

interface AuthZustandStore {
  // State
  isAuthenticated: boolean
  user: AuthUser | null
  isSelfHosted: boolean

  // Actions
  updateUser: (user: Partial<AuthUser>) => void
  setName: (name: string) => void
}

// Initialize from electron store, falling back to the self-hosted user.
// Only cosmetic fields (e.g. name) are user-editable; the id is fixed.
const getInitialUser = (): AuthUser => {
  const storedAuth = window.electron?.store?.get(STORE_KEYS.AUTH) as
    | { user?: AuthUser | null }
    | undefined

  return {
    ...SELF_HOSTED_USER,
    ...(storedAuth?.user?.name ? { name: storedAuth.user.name } : {}),
  }
}

// Sync to electron store
const syncToStore = (user: AuthUser) => {
  if (!window.electron?.store) return
  window.electron.store.set(STORE_KEYS.AUTH, { user })
}

export const useAuthStore = create<AuthZustandStore>((set, get) => ({
  isAuthenticated: true,
  user: getInitialUser(),
  isSelfHosted: true,

  updateUser: (userUpdate: Partial<AuthUser>) => {
    const currentUser = get().user ?? SELF_HOSTED_USER

    // Editable fields come from the update; the identity stays pinned.
    const updatedUser = { ...currentUser, ...userUpdate, ...SELF_HOSTED_USER }

    syncToStore(updatedUser)
    set({ user: updatedUser })
  },

  setName: (name: string) => {
    get().updateUser({ name })
  },
}))

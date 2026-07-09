import { useAuthStore } from '@/app/store/useAuthStore'

/**
 * Single-user local build: there is no login. The app always runs as the
 * fixed self-hosted user, so this hook just exposes that state to keep the
 * call sites simple.
 */
export const useAuth = () => {
  const { isAuthenticated, user, isSelfHosted } = useAuthStore()

  return {
    isAuthenticated,
    user,
    isSelfHosted,
    isLoading: false,
  }
}

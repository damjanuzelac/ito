import { useState } from 'react'
import { useNotesStore } from '../../../../store/useNotesStore'
import { useDictionaryStore } from '../../../../store/useDictionaryStore'
import { useOnboardingStore } from '../../../../store/useOnboardingStore'
import { Button } from '../../../ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../ui/dialog'
import { useAuthStore } from '@/app/store/useAuthStore'

export default function AccountSettingsContent() {
  const { user, setName } = useAuthStore()
  const { loadNotes } = useNotesStore()
  const { loadEntries } = useDictionaryStore()
  const { resetOnboarding } = useOnboardingStore()

  const [showDeleteDialog, setShowDeleteDialog] = useState(false)

  const handleDeleteData = async () => {
    try {
      // Delete all locally stored user data (notes, dictionary, history)
      await window.api.deleteUserData()
    } catch (error) {
      console.error('Failed to delete local data:', error)
      // Still proceed with resetting app state below
    }

    // Clear KV-backed app state
    window.electron.store.set('settings', {})
    window.electron.store.set('main', {})
    window.electron.store.set('onboarding', {})
    window.electron.store.set('auth', {})

    // Reset all stores to their initial state
    resetOnboarding()
    loadNotes()
    loadEntries()

    // Close the dialog
    setShowDeleteDialog(false)
  }

  return (
    <div className="h-full justify-between">
      <div className="space-y-6">
        {/* First name */}
        <div className="flex items-center justify-between">
          <label className="text-sm font-medium text-gray-900">Name</label>
          <input
            type="text"
            value={user?.name ?? ''}
            onChange={e => setName(e.target.value)}
            className="w-80 bg-white border border-gray-300 rounded-lg px-4 py-3 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-transparent"
          />
        </div>
      </div>

      {/* Action buttons */}
      <div className="flex pt-12 w-full justify-center">
        <Button
          variant="ghost"
          size="lg"
          onClick={() => setShowDeleteDialog(true)}
          className="px-6 py-3 text-red-400 hover:text-red-200"
        >
          Delete all data
        </Button>
      </div>

      {/* Delete Confirmation Dialog */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="text-red-600">Delete All Data</DialogTitle>
            <DialogDescription className="text-gray-600">
              Are you absolutely sure you want to delete all locally stored
              data? This action cannot be undone and will permanently remove:
              <br />
              <br />
              • All saved notes
              <br />
              • All dictionary entries
              <br />
              • All dictation history
              <br />
              • All app settings and preferences
              <br />
              <br />
              This will reset Ito to its initial state.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter className="gap-3">
            <Button
              variant="outline"
              onClick={() => setShowDeleteDialog(false)}
            >
              Cancel
            </Button>
            <Button variant="destructive" onClick={handleDeleteData}>
              Yes, delete everything
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

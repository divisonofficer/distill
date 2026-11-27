export type SnackbarTone = 'success' | 'error' | 'loading' | 'undo';
export type SnackbarAction = { label: string; callback: () => void };
export type Snackbar = { id: number; message: string; tone: SnackbarTone; action?: SnackbarAction };

class Ui {
	drawerOpen = $state(false);
	sidebarHidden = $state(false);
	workContext = $state.raw<{ sessionId: string; host: string; workspaceName: string } | null>(null);
	snackbar = $state<Snackbar | null>(null);
	private snackbarId = 0;
	private snackbarTimer: ReturnType<typeof setTimeout> | undefined;
	private snackbarRemaining = 0;
	private snackbarStarted = 0;
	private snackbarPaused = false;
	private snackbarTimerVersion = 0;

	toggleSidebar() {
		this.sidebarHidden = !this.sidebarHidden;
	}

	notify(message: string, tone: SnackbarTone = 'success', action?: SnackbarAction) {
		this.clearSnackbarTimer();
		const id = ++this.snackbarId;
		this.snackbar = { id, message, tone, ...(action ? { action } : {}) };
		this.snackbarRemaining = tone === 'error' || tone === 'loading' ? 0 : tone === 'undo' || action ? 6000 : 2600;
		this.resumeSnackbar();
		return id;
	}

	// A late operation must not replace a newer user action's notification.
	updateSnackbar(id: number, message: string, tone: SnackbarTone = 'success', action?: SnackbarAction) {
		if (this.snackbar?.id !== id) return;
		this.notify(message, tone, action);
	}

	private clearSnackbarTimer() {
		if (this.snackbarTimer !== undefined) clearTimeout(this.snackbarTimer);
		this.snackbarTimer = undefined;
		this.snackbarTimerVersion++;
	}

	pauseSnackbar() {
		this.snackbarPaused = true;
		if (this.snackbarTimer === undefined) return;
		this.snackbarRemaining = Math.max(1, this.snackbarRemaining - (Date.now() - this.snackbarStarted));
		this.clearSnackbarTimer();
	}

	resumeSnackbar() {
		this.snackbarPaused = false;
		if (!this.snackbar || !this.snackbarRemaining || this.snackbarTimer !== undefined) return;
		const id = this.snackbar.id;
		const version = this.snackbarTimerVersion;
		this.snackbarStarted = Date.now();
		this.snackbarTimer = setTimeout(() => {
			if (this.snackbar?.id === id && version === this.snackbarTimerVersion && !this.snackbarPaused) this.dismissSnackbar();
		}, this.snackbarRemaining);
	}

	runSnackbarAction() {
		const current = this.snackbar;
		if (!current?.action) return;
		// Clear first: a callback may publish its own result (or throw).
		this.dismissSnackbar();
		current.action.callback();
	}

	dismissSnackbar() {
		this.clearSnackbarTimer();
		this.snackbarRemaining = 0;
		this.snackbar = null;
	}
}

export const ui = new Ui();

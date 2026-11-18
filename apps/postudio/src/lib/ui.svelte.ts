export type SnackbarTone = 'success' | 'error';
export type SnackbarAction = { label: string; callback: () => void };
export type Snackbar = { id: number; message: string; tone: SnackbarTone; action?: SnackbarAction };

class Ui {
	drawerOpen = $state(false);
	sidebarHidden = $state(false);
	workContext = $state.raw<{ sessionId: string; host: string; workspaceName: string } | null>(null);
	snackbar = $state<Snackbar | null>(null);
	private snackbarId = 0;
	private snackbarTimer: ReturnType<typeof setTimeout> | undefined;

	toggleSidebar() {
		this.sidebarHidden = !this.sidebarHidden;
	}

	notify(message: string, tone: SnackbarTone = 'success', action?: SnackbarAction) {
		if (this.snackbarTimer !== undefined) clearTimeout(this.snackbarTimer);
		const id = ++this.snackbarId;
		this.snackbar = { id, message, tone, ...(action ? { action } : {}) };
		this.snackbarTimer = setTimeout(() => {
			if (this.snackbar?.id === id) this.snackbar = null;
		}, 5000);
	}

	dismissSnackbar() {
		if (this.snackbarTimer !== undefined) clearTimeout(this.snackbarTimer);
		this.snackbarTimer = undefined;
		this.snackbar = null;
	}
}

export const ui = new Ui();

/** Shell layout state: mobile drawer and desktop sidebar visibility. */
class Ui {
	drawerOpen = $state(false);
	sidebarHidden = $state(false);

	toggleSidebar() {
		this.sidebarHidden = !this.sidebarHidden;
	}
}

export const ui = new Ui();

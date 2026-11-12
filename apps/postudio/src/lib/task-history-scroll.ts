export interface ScrollMetrics {
scrollTop: number;
scrollHeight: number;
clientHeight: number;
}

export type HistoryReplayMode = 'initial' | 'reconnect';
export type HistoryScrollTarget = { kind: 'bottom' } | { kind: 'position'; top: number };

const BOTTOM_THRESHOLD = 96;

export function isNearBottom(metrics: ScrollMetrics): boolean {
return metrics.scrollHeight - metrics.clientHeight - metrics.scrollTop <= BOTTOM_THRESHOLD;
}

export class TaskHistoryScroll {
#sessionKey: string | undefined;
#replayed = false;
#following = false;

begin(sessionKey: string): HistoryReplayMode {
if (this.#sessionKey !== sessionKey) {
this.#sessionKey = sessionKey;
this.#replayed = false;
this.#following = false;
}
return this.#replayed ? 'reconnect' : 'initial';
}

complete(mode: HistoryReplayMode, metrics: ScrollMetrics): HistoryScrollTarget {
this.#replayed = true;
this.#following = mode === 'initial' || isNearBottom(metrics);
return this.#following ? { kind: 'bottom' } : { kind: 'position', top: metrics.scrollTop };
}

update(metrics: ScrollMetrics) {
this.#following = this.#replayed && isNearBottom(metrics);
}

shouldFollow(): boolean {
return this.#replayed && this.#following;
}
}

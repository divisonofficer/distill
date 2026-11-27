import { codeCopyText } from './markdown-code';

/** Delegated controls for sanitized renderer output. No inline handlers or execution. */
export function markdownControls(node: HTMLElement, onCitation: (number: number) => void) {
 const timers = new Set<ReturnType<typeof setTimeout>>();
 let disposed = false;
 const onclick = async (event: MouseEvent) => {
  if (!(event.target instanceof Element)) return;
  const link = event.target.closest<HTMLAnchorElement>('a[href]');
  if (link && node.contains(link)) {
   // Apply fixed navigation attributes only after sanitization; never allow extra HTML attributes.
   link.target = '_blank'; link.rel = 'noopener noreferrer'; return;
  }
  const button = event.target.closest<HTMLButtonElement>('button');
  if (!button || !node.contains(button) || button.disabled) return;
  if (button.hasAttribute('data-citation')) { onCitation(Number(button.dataset.citation)); return; }
  const box = button.closest<HTMLElement>('.code-block');
  if (button.hasAttribute('data-wrap') && box) {
   button.setAttribute('aria-pressed', String(box.classList.toggle('code--wrap'))); return;
  }
  if (button.hasAttribute('data-more') && box) {
   const collapsed = box.classList.toggle('code--clamp');
   button.setAttribute('aria-expanded', String(!collapsed));
   button.textContent = collapsed ? `전체 보기 · ${box.dataset.lines}줄` : '접기'; return;
  }
  let source: string | undefined;
  if (button.hasAttribute('data-copy') && box) source = codeCopyText(box.dataset.codeSource ?? '', box.dataset.language ?? '');
  if (button.hasAttribute('data-math-copy')) source = button.closest<HTMLElement>('.math-block')?.dataset.tex;
  if (source === undefined) return;
  const original = button.hasAttribute('data-math-copy') ? 'LaTeX 복사' : '복사';
  try {
   if (!navigator.clipboard?.writeText) throw new Error('Clipboard unavailable');
   await navigator.clipboard.writeText(source);
   if (disposed) return;
   button.textContent = '복사했어요'; button.dataset.done = 'true';
  } catch {
   if (disposed) return;
   button.textContent = '복사 실패'; button.dataset.done = 'false';
  }
  const timer = setTimeout(() => { button.textContent = original; delete button.dataset.done; timers.delete(timer); }, 1500);
  timers.add(timer);
 };
 node.addEventListener('click', onclick);
 return () => { disposed = true; node.removeEventListener('click', onclick); for (const timer of timers) clearTimeout(timer); };
}

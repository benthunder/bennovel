import { useLayoutEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * Backdrop for every dialog and bottom sheet. It is rendered into the app frame, not the
 * screen that opened it: inside a scrolling screen the backdrop sits at the top of the
 * content, so once the page is scrolled down the sheet opens off-screen or half hidden.
 * In the frame it always covers the whole window, above the nav. The sheet inside
 * (`.dialog--sheet`) is capped at the window height and scrolls on its own.
 */
export function Modal({ onClose, children }: { onClose?: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    return () => { if (el) playExit(el); };
  }, []);
  const backdrop = <div ref={ref} className="dialog-backdrop dialog-backdrop--sheet" onClick={onClose}>{children}</div>;
  const host = document.querySelector('.app');
  return host ? createPortal(backdrop, host) : backdrop;
}

const EXIT_MS = 220;

/**
 * Lets a sheet slide away instead of vanishing: callers just stop rendering it, so on unmount
 * a still copy of it is left in its place, plays the closing animation and removes itself.
 * The copy takes no input. StrictMode's trial unmount leaves the real node in the page, and
 * the copy is dropped again before it is painted.
 */
function playExit(el: HTMLElement) {
  const host = el.parentElement;
  if (!host || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  const ghost = el.cloneNode(true) as HTMLElement;
  // A clone starts scrolled to the top: keep each scrolled list where it was.
  const from = el.querySelectorAll<HTMLElement>('*'), to = ghost.querySelectorAll<HTMLElement>('*');
  // A copied checked radio would uncheck its twin in the real sheet (same group name); ids must stay unique.
  ghost.querySelectorAll('[name]').forEach(n => n.removeAttribute('name'));
  ghost.querySelectorAll('[id]').forEach(n => n.removeAttribute('id'));
  ghost.classList.add('is-leaving');
  ghost.setAttribute('aria-hidden', 'true');
  ghost.inert = true;
  host.insertBefore(ghost, el.nextSibling);
  from.forEach((n, i) => { if (n.scrollTop) to[i].scrollTop = n.scrollTop; });
  queueMicrotask(() => {
    if (el.isConnected) { ghost.remove(); return; }
    setTimeout(() => ghost.remove(), EXIT_MS + 40);
  });
}

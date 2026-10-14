import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';

/**
 * Backdrop for every dialog and bottom sheet. It is rendered into the app frame, not the
 * screen that opened it: inside a scrolling screen the backdrop sits at the top of the
 * content, so once the page is scrolled down the sheet opens off-screen or half hidden.
 * In the frame it always covers the whole window, above the nav. The sheet inside
 * (`.dialog--sheet`) is capped at the window height and scrolls on its own.
 */
export function Modal({ onClose, children }: { onClose?: () => void; children: ReactNode }) {
  const backdrop = <div className="dialog-backdrop dialog-backdrop--sheet" onClick={onClose}>{children}</div>;
  const host = document.querySelector('.app');
  return host ? createPortal(backdrop, host) : backdrop;
}

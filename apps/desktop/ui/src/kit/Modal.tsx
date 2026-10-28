import { Dialog } from '@base-ui/react/dialog';
import type { HTMLAttributes, RefObject } from 'react';
import { useSurfaceTheme } from '../theme/ThemeProvider';

export const ModalClose = Dialog.Close;

export interface ModalProps extends HTMLAttributes<HTMLDivElement> {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  returnFocusRef?: RefObject<HTMLElement | null>;
  /**
   * A press outside the window asks to close it, through `onOpenChange`
   * like Escape. Off by default: most windows hold work a stray click must
   * not throw away.
   */
  dismissOnOutsidePress?: boolean;
}

/** Supply an accessible name and a ModalClose button in children. */
export function Modal({
  open,
  onOpenChange,
  returnFocusRef,
  dismissOnOutsidePress = false,
  className = '',
  children,
  ...props
}: ModalProps) {
  const theme = useSurfaceTheme();
  return (
    <Dialog.Root
      open={open}
      onOpenChange={onOpenChange}
      disablePointerDismissal={!dismissOnOutsidePress}
    >
      <Dialog.Portal data-theme={theme}>
        <Dialog.Backdrop className="xt-modal-backdrop" />
        <Dialog.Popup {...props} finalFocus={returnFocusRef} className={`xt-modal ${className}`}>
          {children}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

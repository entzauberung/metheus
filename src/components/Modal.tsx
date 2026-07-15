// src/components/Modal.tsx — 统一弹窗组件（基于 @radix-ui/react-dialog）
import React from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { X } from 'lucide-react';

interface ModalAction {
  label: string;
  onClick: () => void;
  variant?: 'primary' | 'secondary' | 'danger';
  disabled?: boolean;
}

interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
  description?: string;
  showCloseButton?: boolean;
  isDanger?: boolean;
  actions?: ModalAction[];
  /** 提交期间锁定关闭（禁止 Esc、遮罩点击、关闭按钮关闭） */
  lockClose?: boolean;
  /** 是否正在提交（操作按钮显示加载态） */
  isSubmitting?: boolean;
}

export function Modal({
  isOpen,
  onClose,
  title,
  children,
  description,
  showCloseButton = true,
  isDanger = false,
  actions,
  lockClose = false,
  isSubmitting = false,
}: ModalProps) {
  const handleClose = lockClose ? () => {} : onClose;

  return (
    <Dialog.Root open={isOpen} onOpenChange={(open) => { if (!open && !lockClose) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="modal-overlay" />
        <Dialog.Content
          className="modal-content"
          onEscapeKeyDown={handleClose}
          onInteractOutside={handleClose}
        >
          <Dialog.Title className={`modal-title ${isDanger ? 'modal-title-danger' : ''}`}>
            {title}
          </Dialog.Title>
          {description && (
            <Dialog.Description className="modal-description">
              {description}
            </Dialog.Description>
          )}
          {showCloseButton && !lockClose && (
            <Dialog.Close asChild>
              <button className="modal-close-btn" aria-label="关闭">
                <X size={16} />
              </button>
            </Dialog.Close>
          )}
          <div className="modal-body-scrollable">
            {children}
          </div>
          {actions && actions.length > 0 && (
            <div className="modal-actions">
              {actions.map((action, idx) => (
                <button
                  key={idx}
                  className={`modal-action-btn modal-action-${action.variant || 'secondary'}`}
                  onClick={action.onClick}
                  disabled={action.disabled || (isSubmitting && action.variant !== 'secondary')}
                >
                  {action.label}
                </button>
              ))}
            </div>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

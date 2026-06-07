// src/components/Modal.tsx
import React from 'react';

interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
}

export function Modal({ isOpen, onClose, title, children }: ModalProps) {
  if (!isOpen) return null;

  return (
    <div
      onClick={onClose}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,0.4)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 1000,
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: '#fff',
          borderRadius: '8px',
          minWidth: '360px',
          maxWidth: '480px',
          boxShadow: '0 4px 24px rgba(0,0,0,0.2)',
        }}
      >
        <div
          style={{
            padding: '12px 16px',
            borderBottom: '1px solid #eee',
            fontWeight: 600,
            fontSize: '16px',
          }}
        >
          {title}
        </div>
        {children}
      </div>
    </div>
  );
}

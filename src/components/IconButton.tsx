// src/components/IconButton.tsx — 统一图标按钮组件（lucide-react + @radix-ui/react-tooltip）
import React from 'react';
import * as Tooltip from '@radix-ui/react-tooltip';

interface IconButtonProps {
  icon: React.ReactNode;
  tooltip: string;
  onClick?: () => void;
  disabled?: boolean;
  danger?: boolean;
  active?: boolean;
  size?: 'sm' | 'md';
  className?: string;
  ariaLabel?: string;
}

export const ICON_SIZE_SM = 14;
export const ICON_SIZE_MD = 18;

export function IconButton({
  icon,
  tooltip,
  onClick,
  disabled = false,
  danger = false,
  active = false,
  size = 'md',
  className = '',
  ariaLabel,
}: IconButtonProps) {
  const btnClass = [
    'icon-btn',
    `icon-btn-${size}`,
    danger ? 'icon-btn-danger' : '',
    active ? 'icon-btn-active' : '',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <Tooltip.Provider delayDuration={400}>
      <Tooltip.Root>
        <Tooltip.Trigger asChild>
          <button
            className={btnClass}
            onClick={onClick}
            disabled={disabled}
            aria-label={ariaLabel || tooltip}
            type="button"
          >
            {icon}
          </button>
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content className="icon-btn-tooltip" sideOffset={4}>
            {tooltip}
            <Tooltip.Arrow className="icon-btn-tooltip-arrow" />
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>
    </Tooltip.Provider>
  );
}

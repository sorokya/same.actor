/**
 * Inline icons.
 *
 * Six glyphs is not worth a dependency — this also keeps `react-icons` (which imports React)
 * out of the tree, so the whole bundle is Preact with no compat shim underneath it.
 * All are drawn on a 24×24 stroke grid and inherit `currentColor`.
 */

import type { ComponentChildren } from 'preact';

export interface IconProps {
  readonly size?: number;
  readonly className?: string;
}

function Icon({
  size = 16,
  className = '',
  children,
}: IconProps & { children: ComponentChildren }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width={1.6}
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
      focusable="false"
      class={className}
    >
      {children}
    </svg>
  );
}

export function SearchIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <path d="m15.5 15.5 4.5 4.5" />
    </Icon>
  );
}

export function CloseIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m6 6 12 12M18 6 6 18" />
    </Icon>
  );
}

export function FilmIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="3" y="4.5" width="18" height="15" rx="1.5" />
      <path d="M7.5 4.5v15M16.5 4.5v15M3 12h18" />
    </Icon>
  );
}

export function UserIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="12" cy="8.5" r="3.75" />
      <path d="M4.75 20a7.25 7.25 0 0 1 14.5 0" />
    </Icon>
  );
}

export function SpinnerIcon({ className = '', ...props }: IconProps) {
  return (
    <Icon {...props} className={`animate-spin ${className}`}>
      <path d="M12 3a9 9 0 1 0 9 9" />
    </Icon>
  );
}

export function TrashIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M4.5 7h15M9.5 7V4.75h5V7M6.75 7l.9 12.25h8.7L17.25 7" />
      <path d="M10.25 10.75v5.5M13.75 10.75v5.5" />
    </Icon>
  );
}

export function ShareIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M12 3.5v11M12 3.5 8.25 7.25M12 3.5l3.75 3.75" />
      <path d="M5.5 12.5v6.25a1.75 1.75 0 0 0 1.75 1.75h9.5a1.75 1.75 0 0 0 1.75-1.75V12.5" />
    </Icon>
  );
}

export function CheckIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m4.5 12.5 5 5 10-11" />
    </Icon>
  );
}

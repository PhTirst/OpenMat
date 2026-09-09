import type { ReactNode, SVGProps } from "react";

type IconProps = Omit<SVGProps<SVGSVGElement>, "children">;

function Icon({ children, ...props }: IconProps & { readonly children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.35"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {children}
    </svg>
  );
}

export function NewFileIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3.25 1.75h5l3.5 3.5v8.9H3.25z" />
      <path d="M8.25 1.75v3.5h3.5M8 7.25v4.5M5.75 9.5h4.5" />
    </Icon>
  );
}

export function OpenFileIcon(props: IconProps) {
  return <Icon {...props}><path d="M3 2h5l3 3v3M8 2v3h3M3 2v12h5M9 11h5m-2-2 2 2-2 2" /></Icon>;
}

export function FileExplorerIcon(props: IconProps) {
  return <Icon {...props}><path d="M1.5 4h5l1-1.5h4v3M1.5 4v9h10.5V9M1.5 6.5H7M9 6.5h5m-2-2 2 2-2 2" /></Icon>;
}

export function NewFolderIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M1.75 4.1h4l1.2 1.45h7.3v7.2H1.75z" />
      <path d="M9.9 7.25v3.6M8.1 9.05h3.6" />
    </Icon>
  );
}

export function SettingsIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="2.1" />
      <path d="M6.9 1.7h2.2l.35 1.45c.35.12.68.26.98.44l1.28-.76 1.55 1.55-.76 1.28c.18.3.32.63.44.98l1.45.35v2.2l-1.45.35c-.12.35-.26.68-.44.98l.76 1.28-1.55 1.55-1.28-.76c-.3.18-.63.32-.98.44L9.1 14.3H6.9l-.35-1.45a5 5 0 0 1-.98-.44l-1.28.76-1.55-1.55.76-1.28a5 5 0 0 1-.44-.98L1.6 9.1V6.9l1.45-.35c.12-.35.26-.68.44-.98l-.76-1.28 1.55-1.55 1.28.76c.3-.18.63-.32.98-.44z" />
    </Icon>
  );
}

export function FileIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M3.25 1.75h5l3.5 3.5v8.9H3.25z" />
      <path d="M8.25 1.75v3.5h3.5" />
    </Icon>
  );
}

export function FolderIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M1.75 4.1h4l1.2 1.45h7.3v7.2H1.75z" />
    </Icon>
  );
}

export function OpenFolderIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M1.75 5h4l1.2 1.4h7.3l-1.55 6.35H1.75z" />
      <path d="M2.2 5V3.5h4l1.2 1.4h5.4" />
    </Icon>
  );
}

export function ParentFolderIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2 8.1 8 2.4l6 5.7M8 2.8v10.8" />
    </Icon>
  );
}

export function RefreshIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M13.2 5.4A5.7 5.7 0 1 0 13 10.9" />
      <path d="M10.4 5.4h2.8V2.6" />
    </Icon>
  );
}

export function UploadIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 10.8V2.1M4.8 5.3 8 2.1l3.2 3.2" />
      <path d="M2.2 9.2v4.2h11.6V9.2" />
    </Icon>
  );
}

export function BackIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m9.8 3.2-4.7 4.8 4.7 4.8M5.4 8h7.1" />
    </Icon>
  );
}

export function ForwardIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="m6.2 3.2 4.7 4.8-4.7 4.8M10.6 8H3.5" />
    </Icon>
  );
}

export function OtherEntryIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <circle cx="8" cy="8" r="5.25" />
      <path d="M8 5.2v3.3M8 11h.01" />
    </Icon>
  );
}

export function PlotHomeIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M2 7.3 8 2l6 5.3" />
      <path d="M3.7 6.2v7h8.6v-7M6.4 13.2V9h3.2v4.2" />
    </Icon>
  );
}

export function PlotPanIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 1.5v13M5.9 3.6 8 1.5l2.1 2.1M5.9 12.4 8 14.5l2.1-2.1" />
      <path d="M1.5 8h13M3.6 5.9 1.5 8l2.1 2.1M12.4 5.9 14.5 8l-2.1 2.1" />
    </Icon>
  );
}

export function PlotBoxZoomIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <rect x="2" y="2" width="8.5" height="8.5" rx="0.7" />
      <path d="m9.5 9.5 4.3 4.3M4.2 6.25h4.1M6.25 4.2v4.1" />
    </Icon>
  );
}

export function PlotDataCursorIcon(props: IconProps) {
  return (
    <Icon {...props}>
      <path d="M8 1.5v13M1.5 8h13" />
      <circle cx="8" cy="8" r="2.2" fill="var(--figure-canvas)" />
    </Icon>
  );
}

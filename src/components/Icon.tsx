import type { SVGProps } from "react";

const P: Record<string, string> = {
  library: "M4 5h4v14H4z M10 5h4v14h-4z M16.2 6.2l3.4-.9 2.6 12.8-3.4.9z",
  book: "M3 6c3-1.2 6-1.2 9 1 3-2.2 6-2.2 9-1v12c-3-1-6-1-9 1-3-2-6-2-9-1z M12 7v12",
  cards: "M4 8h16v11H4z M7 5h10",
  chart: "M4 20V10 M10 20V4 M16 20v-7 M21 20H3",
  settings:
    "M12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4z M19.4 13.5l1.6 1.2-1.8 3.1-1.9-.7a7.6 7.6 0 0 1-1.7 1l-.3 2H10.7l-.3-2a7.6 7.6 0 0 1-1.7-1l-1.9.7L5 14.7l1.6-1.2a7.4 7.4 0 0 1 0-2L5 10.3l1.8-3.1 1.9.7a7.6 7.6 0 0 1 1.7-1l.3-2h3.6l.3 2a7.6 7.6 0 0 1 1.7 1l1.9-.7 1.8 3.1-1.6 1.2a7.4 7.4 0 0 1 0 2z",
  plus: "M12 5v14 M5 12h14",
  import: "M12 4v11 M7 10l5 5 5-5 M5 20h14",
  search: "M11 17.5a6.5 6.5 0 1 0 0-13 6.5 6.5 0 0 0 0 13z M20 20l-4.2-4.2",
  play: "M8 5.5v13l11-6.5z",
  pause: "M7 5h3.5v14H7z M13.5 5H17v14h-3.5z",
  back: "M15 5l-7 7 7 7",
  forward: "M9 5l7 7-7 7",
  left: "M14.5 6l-6 6 6 6",
  right: "M9.5 6l6 6-6 6",
  close: "M6 6l12 12 M18 6L6 18",
  check: "M5 12.5l4.5 4.5L19 7.5",
  speaker: "M5 9.5h3l4.5-4v13l-4.5-4H5z M16 9a4.2 4.2 0 0 1 0 6 M18.6 6.6a7.6 7.6 0 0 1 0 10.8",
  moon: "M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z",
  sun: "M12 16a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M12 2.5v2 M12 19.5v2 M4.6 4.6l1.4 1.4 M18 18l1.4 1.4 M2.5 12h2 M19.5 12h2 M4.6 19.4L6 18 M18 6l1.4-1.4",
  more: "M6 12h.01 M12 12h.01 M18 12h.01",
  trash: "M5 7h14 M10 7V4.5h4V7 M7 7l1 13h8l1-13",
  edit: "M4 20h4L19 9l-4-4L4 16z M13.5 6.5l4 4",
  text: "M5 6h14 M5 10h14 M5 14h9 M5 18h11",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1 M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  file: "M7 3h7l5 5v13H7z M14 3v5h5",
  wave: "M3 12h2 M7 8v8 M11 5v14 M15 9v6 M19 7v10 M21 12h0",
  video: "M4 6h11v12H4z M15 10l5-3v10l-5-3z",
  sparkle: "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z M19 16l.8 2.2L22 19l-2.2.8L19 22l-.8-2.2L16 19l2.2-.8z",
  rewind: "M11 7l-6 5 6 5z M19 7l-6 5 6 5z",
  ffwd: "M13 7l6 5-6 5z M5 7l6 5-6 5z",
  back5: "M4 12a8 8 0 1 0 2.4-5.7 M4 4v4h4",
  fwd5: "M20 12a8 8 0 1 1-2.4-5.7 M20 4v4h-4",
  chevron: "M7 10l5 5 5-5",
  download: "M12 4v11 M7 10l5 5 5-5 M5 20h14",
  cpu: "M7 7h10v10H7z M10 3v4 M14 3v4 M10 17v4 M14 17v4 M3 10h4 M3 14h4 M17 10h4 M17 14h4",
  globe: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M3 12h18 M12 3c2.5 2.7 2.5 15.3 0 18 M12 3c-2.5 2.7-2.5 15.3 0 18",
  type: "M5 7V5h14v2 M12 5v14 M9 19h6",
  layers: "M12 4l9 5-9 5-9-5z M3 14l9 5 9-5",
  flame: "M12 21c-4 0-6.5-2.6-6.5-6.2 0-3.6 3-5.6 3.5-9.3 2.4 1.6 3.4 3.4 3.5 5.5 1-.7 1.6-1.8 1.7-3.2 2.4 1.9 4.3 4.4 4.3 7.1C18.5 18.4 16 21 12 21z",
  clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 7v5l3.5 2",
  eye: "M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  ban: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M5.6 5.6l12.8 12.8",
  youtube: "M3 8.5c0-1.6 1.1-2.8 2.7-2.9C7.6 5.4 9.8 5.3 12 5.3s4.4.1 6.3.3c1.6.1 2.7 1.3 2.7 2.9v7c0 1.6-1.1 2.8-2.7 2.9-1.9.2-4.1.3-6.3.3s-4.4-.1-6.3-.3C4.1 18.3 3 17.1 3 15.5z M10 9v6l5-3z",
  export: "M12 15V4 M7 9l5-5 5 5 M5 14v6h14v-6",
  external: "M14 4h6v6 M20 4l-9 9 M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
  image: "M4 7a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z M4 16l4.5-4.5 4 4 2.5-2.5L20 18 M15.5 9h.01",
};

export type IconName = keyof typeof P;

export function Icon({
  name,
  size = 18,
  stroke = 1.7,
  ...rest
}: { name: IconName; size?: number; stroke?: number } & Omit<SVGProps<SVGSVGElement>, "stroke">) {
  const filled = name === "play" || name === "pause";
  return (
    <svg
      className="icon"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill={filled ? "currentColor" : "none"}
      stroke={filled ? "none" : "currentColor"}
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {P[name].split(" M").map((d, i) => (
        <path key={i} d={i === 0 ? d : "M" + d} />
      ))}
    </svg>
  );
}

import { useState } from 'preact/hooks';
import { FilmIcon, UserIcon } from './icons';

export interface PosterProps {
  /** `null` when TMDB has no artwork. The placeholder is ours, not a fake URL. */
  src: string | null;
  alt: string;
  shape?: 'poster' | 'avatar';
  /** Fixed size. Omit both and set `fluid` for a cell that follows its container. */
  width?: number;
  height?: number;
  /**
   * Fill the container, keeping the shape's aspect ratio (2:3 for a poster, 1:1 for an avatar).
   * Needed wherever the artwork sits in a responsive grid, where a pixel width would either
   * overflow a phone or leave a gap on a desktop.
   */
  fluid?: boolean;
  className?: string;
}

/**
 * A fixed-size image well.
 *
 * Dimensions are always supplied, so the browser reserves the space and nothing shifts when
 * the image lands (review M6). Images are lazy: a 16-result dropdown used to fetch every
 * thumbnail at full size immediately.
 */
export function Poster({
  src,
  alt,
  shape = 'poster',
  width,
  height,
  fluid = false,
  className = '',
}: PosterProps) {
  // Track which source failed rather than a boolean, so a new url is never shown broken and
  // no effect is needed to reset the flag.
  const [failedSrc, setFailedSrc] = useState<string | null>(null);

  const show = src !== null && src !== '' && failedSrc !== src;
  const radius = shape === 'avatar' ? 'rounded-full' : 'rounded-[3px]';

  // Icon sizing only needs a rough scale; a fluid cell uses a sensible default.
  const iconWidth = width ?? (shape === 'avatar' ? 40 : 100);

  if (fluid) {
    return (
      <span
        className={`flex items-center justify-center overflow-hidden border border-line-soft bg-raise-2 ${
          shape === 'avatar' ? 'aspect-square' : 'aspect-[2/3]'
        } w-full ${radius} ${className}`}
      >
        {show ? (
          <img
            src={src}
            alt={alt}
            loading="lazy"
            decoding="async"
            className="h-full w-full object-cover"
            onError={() => setFailedSrc(src)}
          />
        ) : (
          <span aria-hidden="true" className="text-dim/60">
            {shape === 'avatar' ? (
              <UserIcon size={Math.round(iconWidth * 0.42)} />
            ) : (
              <FilmIcon size={Math.round(iconWidth * 0.45)} />
            )}
          </span>
        )}
      </span>
    );
  }

  // Units are explicit because Preact 11 removed the automatic `px` it used to append to
  // numeric style values: `{ width: 22 }` is assigned verbatim to CSSStyleDeclaration, "22"
  // is not a valid width, and the declaration is then dropped without any warning.
  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center overflow-hidden border border-line-soft bg-raise-2 ${radius} ${className}`}
      style={{ width: `${width ?? 0}px`, height: `${height ?? 0}px` }}
    >
      {show ? (
        <img
          src={src}
          alt={alt}
          width={width}
          height={height}
          loading="lazy"
          decoding="async"
          className="h-full w-full object-cover"
          onError={() => setFailedSrc(src)}
        />
      ) : (
        <span aria-hidden="true" className="text-dim/60">
          {shape === 'avatar' ? (
            <UserIcon size={Math.round(iconWidth * 0.42)} />
          ) : (
            <FilmIcon size={Math.round(iconWidth * 0.45)} />
          )}
        </span>
      )}
    </span>
  );
}

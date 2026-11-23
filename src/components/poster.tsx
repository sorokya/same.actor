import { useState } from 'react';
import { FaFilm, FaUser } from 'react-icons/fa';

export interface PosterProps {
  /** `null` when TMDB has no artwork. The placeholder is ours, not a fake URL. */
  src: string | null;
  alt: string;
  shape?: 'poster' | 'avatar';
  width: number;
  height: number;
  className?: string;
}

/**
 * A fixed-size image well.
 *
 * Dimensions are always supplied, so the browser reserves the space and nothing shifts when
 * the image lands (review M6). Images are lazy: a 16-result dropdown used to fetch every
 * thumbnail at full size immediately.
 */
export function Poster({ src, alt, shape = 'poster', width, height, className = '' }: PosterProps) {
  // Track which source failed rather than a boolean, so a new url is never shown broken and
  // no effect is needed to reset the flag.
  const [failedSrc, setFailedSrc] = useState<string | null>(null);

  const show = src !== null && src !== '' && failedSrc !== src;
  const radius = shape === 'avatar' ? 'rounded-full' : 'rounded-[3px]';

  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center overflow-hidden border border-line-soft bg-raise-2 ${radius} ${className}`}
      style={{ width, height }}
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
            <FaUser size={Math.round(width * 0.42)} />
          ) : (
            <FaFilm size={Math.round(width * 0.45)} />
          )}
        </span>
      )}
    </span>
  );
}

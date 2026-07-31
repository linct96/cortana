import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function appError(error: unknown) {
  return typeof error === 'string'
    ? error
    : error instanceof Error
      ? error.message
      : '操作没有完成。';
}

export function formatResetTime(value: number, now = Date.now()) {
  const totalMinutes = Math.max(0, Math.ceil((value - now) / 60_000));
  const days = Math.floor(totalMinutes / 1_440);
  const hours = Math.floor((totalMinutes % 1_440) / 60);
  const minutes = totalMinutes % 60;

  if (days) return `${days}d ${hours}h 后重置`;
  if (hours) return `${hours}h ${minutes}m 后重置`;
  return `${minutes}m 后重置`;
}

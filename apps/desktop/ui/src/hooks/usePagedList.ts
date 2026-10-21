import { useQueries, type QueryKey } from "@tanstack/react-query";

export const LIST_PAGE = 128;

/**
 * The visible window of an append-only list held in Rust (WebSocket
 * messages, SSE events), fetched a page at a time. A page's key includes
 * how many of its slots are filled, so full pages are fetched once and only
 * the tail page refetches as the list grows; meanwhile its previous copy is
 * shown, so live updates never flash empty rows.
 */
export function usePagedList<T>(
  key: QueryKey,
  len: number,
  first: number,
  last: number,
  fetchPage: (start: number, count: number) => Promise<T[]>,
): (index: number) => T | undefined {
  const pages: number[] = [];
  if (len > 0 && last >= first) {
    const lastPage = Math.floor(Math.min(last, len - 1) / LIST_PAGE);
    for (let p = Math.floor(Math.max(0, first) / LIST_PAGE); p <= lastPage; p++) pages.push(p);
  }
  const prefix = JSON.stringify(key);
  const results = useQueries({
    queries: pages.map((page) => ({
      queryKey: [...key, page, Math.min(len, (page + 1) * LIST_PAGE)],
      queryFn: () => fetchPage(page * LIST_PAGE, LIST_PAGE),
      // Only an earlier copy of this same page stands in.
      placeholderData: (previous: T[] | undefined, query?: { queryKey: QueryKey }) =>
        query && query.queryKey.at(-2) === page && JSON.stringify(query.queryKey.slice(0, -2)) === prefix
          ? previous
          : undefined,
    })),
  });
  return (index: number) => {
    const slot = pages.indexOf(Math.floor(index / LIST_PAGE));
    return results[slot]?.data?.[index % LIST_PAGE];
  };
}

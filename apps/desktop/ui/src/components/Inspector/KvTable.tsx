export type Kv = [string, string][];

export function KvTable({ rows }: { rows: Kv }) {
  return (
    <table className="w-full font-mono text-xs" data-selectable>
      <tbody>
        {rows.map(([k, v], i) => (
          <tr key={`${k}-${i}`} className="border-border/50 border-b">
            <td className="w-1/3 px-2 py-1 align-top font-medium text-[var(--syntax-key)]">{k}</td>
            <td className="px-2 py-1 break-all">{v}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function Placeholder({ text }: { text: string }) {
  return <div className="text-muted-foreground p-4 text-center text-xs">{text}</div>;
}

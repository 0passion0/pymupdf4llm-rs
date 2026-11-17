import React from 'react';
import { parseMarkdownToBlocks } from '../engine/markdownPreview';

interface MarkdownRendererProps {
  markdown: string;
}

function renderInline(text: string, insideLink = false): React.ReactNode[] {
  // Tokenize links, inline code, bold-italic, bold, italic
  const pattern = /(\[[^\]]+\]\([^)]+\)|`[^`]+`|\*\*\*[^*]+\*\*\*|\*\*[^*]+\*\*|\*[^*]+\*)/g;
  const parts = text.split(pattern);
  return parts.map((part, idx) => {
    if (!part) return null;
    const linkMatch = !insideLink ? part.match(/^\[([^\]]+)\]\(([^)]+)\)$/) : null;
    if (linkMatch) {
      return (
        <a
          key={idx}
          href={linkMatch[2]}
          target="_blank"
          rel="noopener noreferrer"
          className="font-medium text-blue-600 underline decoration-blue-300 underline-offset-2 hover:text-blue-700"
        >
          {renderInline(linkMatch[1], true)}
        </a>
      );
    }
    if (part.startsWith('`') && part.endsWith('`') && part.length >= 2) {
      return (
        <code
          key={idx}
          className="rounded bg-slate-100 px-1.5 py-0.5 font-mono text-[13px] text-slate-800"
        >
          {part.slice(1, -1)}
        </code>
      );
    }
    if (part.startsWith('***') && part.endsWith('***') && part.length >= 6) {
      return (
        <strong key={idx} className="font-semibold italic text-slate-900">
          {part.slice(3, -3)}
        </strong>
      );
    }
    if (part.startsWith('**') && part.endsWith('**') && part.length >= 4) {
      return (
        <strong key={idx} className="font-semibold text-slate-900">
          {part.slice(2, -2)}
        </strong>
      );
    }
    if (part.startsWith('*') && part.endsWith('*') && part.length >= 2) {
      return (
        <em key={idx} className="italic text-slate-700">
          {part.slice(1, -1)}
        </em>
      );
    }
    return <React.Fragment key={idx}>{part}</React.Fragment>;
  });
}

export function MarkdownRenderer({ markdown }: MarkdownRendererProps) {
  const blocks = parseMarkdownToBlocks(markdown);

  if (blocks.length === 0) {
    return (
      <div className="py-12 text-center text-sm text-slate-500">
        No Markdown output generated for the current page selection or margin settings.
      </div>
    );
  }

  return (
    <article className="space-y-4 text-[15px] leading-relaxed text-slate-700">
      {blocks.map((block, idx) => {
        if (block.type === 'heading') {
          const sizes: Record<number, string> = {
            1: 'text-2xl font-bold tracking-tight text-slate-900 pt-2',
            2: 'text-xl font-semibold tracking-tight text-slate-900 pt-2',
            3: 'text-lg font-semibold text-slate-900 pt-1',
            4: 'text-base font-semibold text-slate-900',
            5: 'text-sm font-semibold text-slate-800',
            6: 'text-sm font-medium text-slate-700',
          };
          return (
            <h2 key={idx} className={sizes[block.level] ?? sizes[3]}>
              {renderInline(block.text)}
            </h2>
          );
        }
        if (block.type === 'code') {
          return (
            <pre
              key={idx}
              className="overflow-x-auto rounded-lg border border-slate-200 bg-slate-900 p-4 font-mono text-[13px] leading-relaxed text-slate-100"
            >
              <code>{block.code}</code>
            </pre>
          );
        }
        if (block.type === 'hr') {
          return (
            <div key={idx} className="my-6 flex items-center gap-3">
              <hr className="flex-1 border-slate-200" />
              <span className="font-mono text-xs text-slate-400">Page Break</span>
              <hr className="flex-1 border-slate-200" />
            </div>
          );
        }
        if (block.type === 'image') {
          return (
            <figure key={idx} className="overflow-hidden rounded-lg border border-slate-200 bg-slate-50 p-3">
              <img
                src={block.src}
                alt={block.alt || 'Extracted PDF image'}
                referrerPolicy="no-referrer"
                className="max-h-48 rounded border border-slate-200 bg-white object-contain"
              />
              {block.alt && <figcaption className="mt-2 text-xs text-slate-500">{block.alt}</figcaption>}
            </figure>
          );
        }
        if (block.type === 'list') {
          return (
            <ul key={idx} className="list-disc space-y-1.5 pl-5 text-slate-700">
              {block.items.map((item, ii) => (
                <li key={ii}>{renderInline(item)}</li>
              ))}
            </ul>
          );
        }
        if (block.type === 'table') {
          return (
            <div key={idx} className="overflow-x-auto rounded-lg border border-slate-200">
              <table className="w-full border-collapse text-left text-sm">
                <thead className="border-b border-slate-200 bg-slate-50">
                  <tr>
                    {block.headers.map((h, hi) => (
                      <th key={hi} className="px-3.5 py-2.5 font-semibold text-slate-900">
                        {renderInline(h)}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-200 bg-white">
                  {block.rows.map((row, ri) => (
                    <tr key={ri} className="hover:bg-slate-50">
                      {row.map((cell, ci) => (
                        <td key={ci} className="px-3.5 py-2 font-mono text-[13px] tabular-nums text-slate-700">
                          {renderInline(cell)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          );
        }
        return (
          <p key={idx} className="max-w-[74ch]">
            {renderInline(block.text)}
          </p>
        );
      })}
    </article>
  );
}

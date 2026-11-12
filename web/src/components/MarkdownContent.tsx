import { memo, useLayoutEffect } from "react";
import Markdown from "react-markdown";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkGfm from "remark-gfm";

export interface MarkdownContentProps {
  text: string;
  className?: string;
  /** 内容完成布局后通知父级，用于懒加载完成后重新贴底。 */
  onRendered?: () => void;
}

/**
 * 在默认白名单上禁用图片：智能体报告不需要远程图片，
 * 同时避免 Markdown 图片造成外部请求与跟踪像素。
 */
const sanitizeSchema = {
  ...defaultSchema,
  tagNames: (defaultSchema.tagNames ?? []).filter((tag) => tag !== "img"),
};

/**
 * 智能体长报告的 Markdown 渲染。
 *
 * 只启用 GFM 与 sanitize，不解析原始 HTML；链接统一在新标签打开。
 * 表格包一层可横向滚动的容器，避免窄屏撑破消息气泡。
 */
function MarkdownContent({
  text,
  className,
  onRendered,
}: MarkdownContentProps) {
  useLayoutEffect(() => {
    onRendered?.();
  }, [onRendered, text]);

  const classes =
    className === undefined ? "markdown-body" : `markdown-body ${className}`;
  return (
    <div className={classes}>
      <Markdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeSanitize, sanitizeSchema]]}
        components={{
          a: ({ node: _node, ...props }) => (
            <a {...props} target="_blank" rel="noopener noreferrer" />
          ),
          table: ({ node: _node, ...props }) => (
            <div className="markdown-table-wrap">
              <table {...props} />
            </div>
          ),
        }}
      >
        {text}
      </Markdown>
    </div>
  );
}

// 父页面在流式增量时会频繁重渲染；终态消息内容不变时跳过 Markdown 重解析。
export default memo(MarkdownContent);

import type { HTMLAttributes, ReactNode } from "react";

export interface PanelProps extends HTMLAttributes<HTMLElement> {
  eyebrow?: string;
  title?: string;
  children: ReactNode;
}

export function Panel({ eyebrow, title, children, className, ...rest }: PanelProps) {
  const classes = ["panel", className].filter(Boolean).join(" ");
  return (
    <section className={classes} {...rest}>
      {(eyebrow || title) && (
        <div className="panel-header">
          <div>
            {eyebrow && <p className="eyebrow">{eyebrow}</p>}
            {title && <h2>{title}</h2>}
          </div>
        </div>
      )}
      {children}
    </section>
  );
}

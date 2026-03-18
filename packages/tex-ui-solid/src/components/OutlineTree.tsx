import { For, Show } from "solid-js";
import { HEADING_LABELS, type OutlineNode } from "../lib/outline";

type OutlineTreeProps = {
  nodes: OutlineNode[];
  collapsedNodes: Set<number>;
  onToggle: (blockIndex: number) => void;
  onClick: (blockIndex: number) => void;
};

type OutlineItemProps = Omit<OutlineTreeProps, "nodes"> & {
  node: OutlineNode;
};

function OutlineItem(props: OutlineItemProps) {
  const hasChildren = () => props.node.children.length > 0;
  const isCollapsed = () => props.collapsedNodes.has(props.node.blockIndex);
  const levelLabel = () => HEADING_LABELS[props.node.level] ?? `H${props.node.level}`;

  return (
    <li class="outline-item" data-level={props.node.level}>
      <div class="outline-item-row">
        <button
          class="outline-chevron"
          classList={{ collapsed: isCollapsed(), invisible: !hasChildren() }}
          onClick={(event) => {
            event.stopPropagation();
            props.onToggle(props.node.blockIndex);
          }}
          type="button"
          tabIndex={-1}
        >
          <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
            <path d="M3 2 L7 5 L3 8 Z" />
          </svg>
        </button>
        <button
          class="outline-item-button"
          onClick={() => props.onClick(props.node.blockIndex)}
          type="button"
          title={props.node.text}
        >
          <span class="outline-level-badge" data-level={props.node.level}>
            {levelLabel()}
          </span>
          <span class="outline-item-text">{props.node.text}</span>
        </button>
      </div>
      <Show when={hasChildren() && !isCollapsed()}>
        <OutlineTree
          nodes={props.node.children}
          collapsedNodes={props.collapsedNodes}
          onToggle={props.onToggle}
          onClick={props.onClick}
        />
      </Show>
    </li>
  );
}

export function OutlineTree(props: OutlineTreeProps) {
  return (
    <ul class="outline-list">
      <For each={props.nodes}>
        {(node) => (
          <OutlineItem
            node={node}
            collapsedNodes={props.collapsedNodes}
            onToggle={props.onToggle}
            onClick={props.onClick}
          />
        )}
      </For>
    </ul>
  );
}

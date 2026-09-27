// SPDX-License-Identifier: Apache-2.0
// Fixture for the extraction tests: React components written in TSX.
import React, { useState, useEffect } from 'react';
import type { ReactNode } from 'react';
import { Button } from "./components/Button";
import styles from './List.module.css';

export interface ListProps<T> {
  items: T[];
  render(item: T): ReactNode;
  onSelect?: (item: T) => void;
}

type State = { selected: number | null; loading: boolean };

const PAGE_SIZE = 20;

/** Renders a selectable list. */
export function List<T>(props: ListProps<T>): JSX.Element {
  const [state, setState] = useState<State>({ selected: null, loading: false });

  useEffect(() => {
    fetchPage(PAGE_SIZE).then((rows) => setState({ selected: null, loading: false }));
  }, []);

  return (
    <ul className={styles.list}>
      {props.items.map((item, i) => (
        <li key={i}>
          <Button onClick={() => props.onSelect?.(item)}>{props.render(item)}</Button>
        </li>
      ))}
    </ul>
  );
}

/**
 * A class component with an error boundary.
 */
export class Boundary extends React.Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  private log(error: Error): void {
    console.error(error.message);
  }

  componentDidCatch(error: Error) {
    this.log(error);
  }

  render() {
    return this.state.failed ? <Fallback message="failed" /> : this.props.children;
  }
}

export const Fallback = ({ message }: { message: string }) => <p role="alert">{message}</p>;

const helper = (n: number): number => n * 2;

export default function App() {
  return (
    <Boundary>
      <List items={[1, 2, 3]} render={(n) => <span>{helper(n)}</span>} />
    </Boundary>
  );
}

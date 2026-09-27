// SPDX-License-Identifier: Apache-2.0
// Fixture for the extraction tests: a typed repository layer.
import type { User } from './models';
import { Logger } from "./logger";
import * as util from 'util';
import legacy = require("legacy-lib");

/** Something with an identity. */
export interface Entity {
  id: number;
  describe(): string;
}

export interface Repo<T extends Entity> extends Base, Disposable {
  find(id: number): Promise<T | undefined>;
  save(item: T): void;
}

export type Id = string | number;
type Callback = (err: Error | null) => void;

export enum Role {
  Admin,
  Guest,
}

export namespace Validation {
  export function check(user: User): boolean {
    return user.name.length > 0;
  }
  function internal() {}
}

/**
 * An in-memory repository.
 */
@Injectable({ providedIn: 'root' })
export abstract class MemoryRepo<T extends Entity> extends Store<T> implements Repo<T>, Loggable {
  private readonly items = new Map<number, T>();
  protected static count = 0;

  constructor(private logger: Logger, public name: string) {
    super();
  }

  /** Finds one item. */
  public async find(id: number): Promise<T | undefined> {
    this.logger.debug(`find ${id}`);
    return this.items.get(id);
  }

  save(item: T): void {
    this.items.set(item.id, item);
    util.inspect(item);
  }

  private secret(value: Id): Result<Id> {
    return compute(value);
  }

  protected abstract validate(item: T): boolean;

  get length(): number {
    return this.items.size;
  }

  #hidden() {}
}

export function over(a: string): string;
export function over(a: number): number;
export function over(a: any): any {
  return a;
}

export const handler = async (req: Request): Promise<Response> => {
  return new Response(String(req.url));
};

function local<T>(value: T): T {
  return value;
}

// SPDX-License-Identifier: Apache-2.0
// Fixture for the extraction tests: a small event bus in JavaScript.
import fs, { readFile as read } from 'node:fs';
import * as path from "path";
import './polyfills.js';
const lodash = require('lodash');

export { helper as renamed } from './helper.js';

/** Default number of listeners. */
export const DEFAULT_LIMIT = 10;
const internalCounter = 0;
let mutableState = null;

/**
 * Base class of every emitter.
 */
export class Emitter {
  #listeners = new Map();
  static instances = 0;

  /** Creates the emitter. */
  constructor(name) {
    this.name = name;
    Emitter.instances++;
  }

  /**
   * Registers a listener.
   * @param {string} event the event name
   */
  on(event, callback) {
    this.#store(event).push(callback);
    return this;
  }

  static create(name) {
    return new Emitter(name);
  }

  get size() {
    return this.#listeners.size;
  }

  #store(event) {
    return this.#listeners.get(event) ?? [];
  }

  emit = (event, ...args) => {
    for (const cb of this.#store(event)) cb(...args);
  };
}

export default class Bus extends Emitter {
  publish(topic, payload) {
    super.on(topic, payload);
    return path.join(topic, String(payload));
  }
}

/** Adds two numbers. */
export function add(a, b) {
  return helper(a) + Math.max(a, b);
}

export const double = async (x) => {
  const value = await compute(x);
  return value * 2;
};

const memo = function cached(key) { return lodash.get(key); };

function privateUtil() {
  return fs.readFileSync('x');
}

function* ids() {
  yield 1;
}

exports.legacy = function () { return 1; };
module.exports = { add };

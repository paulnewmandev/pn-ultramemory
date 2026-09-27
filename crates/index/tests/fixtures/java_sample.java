// SPDX-License-Identifier: Apache-2.0
package com.example.shop;

import java.util.List;
import java.util.Map;
import static java.lang.Math.max;
import com.example.model.*;

/**
 * A shopping cart.
 *
 * <p>Holds items until checkout.
 */
@Entity
@Table(name = "carts")
public class Cart<T extends Item> extends AbstractCart implements Comparable<Cart>, Serializable {
    public static final int MAX_ITEMS = 50;
    private static final String PREFIX = "cart-";
    private final List<T> items;
    protected int version;

    /** Creates an empty cart. */
    public Cart() {
        this.items = new ArrayList<>();
    }

    /**
     * Adds an item.
     *
     * @param item the item to add
     * @return true when added
     */
    @Override
    public boolean add(T item) {
        validate(item);
        return items.add(item) && Logger.info("added", item);
    }

    private static <U extends Item> List<U> copyOf(Map<String, U> source, int limit) throws IOException {
        Repository repo = new Repository();
        return repo.load(source, max(limit, 1));
    }

    abstract void validate(T item);

    int size() { return items.size(); }

    public enum Status { OPEN, CLOSED }

    public interface Visitor extends Base {
        void visit(Item item);
        default void done() {}
    }

    record Pair(int left, int right) implements Comparable<Pair> {}

    @interface Marker { String value(); }
}

interface Priced<T> {
    T price();
}

enum Color {
    RED, GREEN;

    String label() { return name().toLowerCase(); }
}

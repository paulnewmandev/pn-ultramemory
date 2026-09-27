// SPDX-License-Identifier: Apache-2.0
package com.example.shop

import kotlin.math.max
import java.util.UUID

const val VERSION = "1.0"

/**
 * A shopping cart.
 * Holds items until checkout.
 */
@Service
open class Cart(private val owner: String) : Base(), Comparable<Cart> {
    private val items = mutableListOf<Item>()

    // Adds an item and returns the new size.
    fun add(item: Item): Int {
        val label = "brace { in a string"
        validate(item)
        items.add(item)
        return max(items.size, 1) + format(label).length
    }

    private suspend fun load(
        id: UUID,
        refresh: Boolean = false
    ): Item {
        return repository.find(id) ?: throw NotFound(id)
    }

    override fun compareTo(other: Cart) = items.size - other.items.size

    companion object {
        const val LIMIT = 10
        fun create(owner: String) = Cart(owner)
    }
}

interface Priced {
    fun price(): Long
    fun discounted(rate: Double): Long = (price() * rate).toLong()
}

data class Point(val x: Int, val y: Int)

enum class Color { RED, GREEN }

sealed class Result {
    class Ok(val value: Int) : Result()
    object Failure : Result()
}

fun topLevel(a: Int): Int = a + helper(a)

/* Block comment documentation. */
internal fun helper(a: Int): Int {
    return a * 2
}

object Registry {
    val items = mutableListOf<String>()
    fun register(name: String) { items.add(name) }
}

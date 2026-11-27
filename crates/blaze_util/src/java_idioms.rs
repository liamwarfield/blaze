//! Classes of `lib/util` and `lib/collect` that stand for a Java idiom Rust has in the language or
//! in `std`, or that serve only the JVM. Their uses are ported with the Rust form; the lines below
//! name, per Java method, what plays its role.

// PORT: skipped, Rust enums (Result or an enum of the use) at each of the 42 uses: Either.ofLeft, Either.ofRight, Either.consume, Either.map
// PORT: skipped, Rust enums (Result or an enum of the use) at each of the 42 uses: Either.hashCode, Either.equals, Either.toString
// PORT: skipped, Result (derive(PartialEq, Hash, Debug) where needed): ValueOrException.ofValue, ValueOrException.ofException, ValueOrException.isPresent
// PORT: skipped, Result (derive(PartialEq, Hash, Debug) where needed): ValueOrException.get, ValueOrException.getUnchecked, ValueOrException.getException
// PORT: skipped, Result (derive(PartialEq, Hash, Debug) where needed): ValueOrException.toString, ValueOrException.equals, ValueOrException.hashCode
// PORT: skipped, Rust tuples: Pair.of, Pair.getFirst, Pair.getSecond, Pair.equals, Pair.hashCode
// PORT: skipped, Rust has no null (Option and std collections at the uses): CollectionUtils.asSetWithoutNulls, CollectionUtils.copyOf, CollectionUtils.isNullOrEmpty
// PORT: skipped, CharSequence plumbing (the ported uses render with Display): OnDemandString.charAt, OnDemandString.length, OnDemandString.subSequence
// PORT: skipped, JVM-only (jdk.internal.misc.Blocker for virtual threads around blocking calls): Blocker.begin, Blocker.end, Blocker.blockerType
// PORT: skipped, JVM-only (jdk.internal.misc.Blocker for virtual threads around blocking calls): Blocker.getEnd, Blocker.getBegin
// PORT: skipped, JVM-only (FileChannel interruptibility; Rust I/O is never closed by an interrupt): FileChannels.setUninterruptible
// PORT: skipped, unused at the pin (no other class refers to it): PathFragmentFilter.convert, PathFragmentFilter.getTypeDescription
// PORT: skipped, unused at the pin (no other class refers to it): PathFragmentFilter.isIncluded, PathFragmentFilter.toString
// PORT: skipped, unused at the pin (no other class refers to it): ByteStringDeterministicWriter.writeTo, ByteStringDeterministicWriter.getBytes
// PORT: skipped, no caller at the pin: UserUtils.getOriginatingUser
// PORT: skipped, test-only (VisibleForTesting hostname override): NetUtil.overrideHostnameSupplier

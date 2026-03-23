-------------------------------- MODULE futex --------------------------------

EXTENDS Integers, Sequences, TLC

CONSTANT Proc
CONSTANT Null
CONSTANT MaxSignalDepth
CONSTANT MaxIterations

Symmetry == Permutations(Proc)

(*********
PlusCal options (wf)

Lock and unlock the mutex at least MaxIterations times while allowing signals
to interrupt and also try to take the mutex. If MaxIterations, MaxSignalDepth,
and Proc are finite, and the processes are fair, then this should terminate.
I.e. there are no deadlocks and we always eventually make progress when threads
eventually run.

We have made some concessions to limit state space and make it feasible to
check more complex configurations.

* The main loop and signal handler both check MaxIterations and decrement
  iterations. This is much faster than using separate counters. The lock may be
  harmlessly taken more than MaxIterations times if signals interrupt after
  checking iterations>0.
* In real life, a signal could interrupt at any instruction, but in this
  algorithm, the signals only interrupt at hand-selected locations of interest.
* Atomic orderings aren't modeled (Acquire, Release, Relaxed, SeqCst).

 --fair algorithm Mutex 
 { variables
    holder_tid=Null,
    waiters_flag=FALSE,
    waiters_set={};
    is_waiting = [ i \in Proc |-> FALSE ];
    in_critical_section = FALSE;
   macro futex_wake() {
     if (waiters_set # {}) {
       with (p \in waiters_set) {
         waiters_set := waiters_set \ {p};
         is_waiting[p] := FALSE;
       }
     }
   }
   macro maybe_signal() {
     if (iterations > 0 /\ signal_depth < MaxSignalDepth) {
       either {
         skip;
       } or {
         call handle_signal();
       };
     }
   }
   procedure handle_signal()
   {
   s0:  signal_depth := signal_depth + 1;
        call lock_unlock();
   s1:  signal_depth := signal_depth - 1;
        return;
   }
   procedure lock_unlock()
     variable got_tid=Null, got_flag=FALSE;
   {
           \* lock
   l0:     if (holder_tid = Null /\ waiters_flag = FALSE) {
             holder_tid := self;
           } else {
             got_tid := holder_tid;
             got_flag := waiters_flag;

             \* lock_contended
   l_cnt:    maybe_signal();
   l1:       if (got_tid = self) {
               assert signal_depth > 0;
               return; \* Err(Recursive)
             };
   l2:       if (got_tid = Null) {
               if (holder_tid = got_tid /\ waiters_flag = got_flag) {
                 holder_tid := self;
				 waiters_flag := TRUE; \* NEW
                 goto cs;
               } else {
                 got_tid := holder_tid;
                 got_flag := waiters_flag;
                 goto l_cnt;
               }
             };
   l3:       maybe_signal();
   l4:       if (got_flag = FALSE) {
               if (waiters_flag = got_flag /\ holder_tid = got_tid) {
                 waiters_flag := TRUE;
				 got_flag := TRUE;
               } else {
                 got_tid := holder_tid;
                 got_flag := waiters_flag;
                 goto l_cnt;
               }
             };
   l5:       maybe_signal();
             \* futex_wait
   w0:       if (holder_tid = got_tid /\ waiters_flag = got_flag) {
               waiters_set := waiters_set \union { self };
               is_waiting[self] := TRUE;
   w1:         await is_waiting[self] = FALSE;
             };
   l6:       maybe_signal();
   l7:       got_tid := holder_tid; got_flag := waiters_flag;
             goto l_cnt;
           };

           \* critical section
           \*   We use a assert on a variable instead of a safety property on
           \*   pc["cs"] since we want a Proc to also be mutally excluded with
           \*   its own signal handlers.
   cs:     assert ~in_critical_section;
           in_critical_section := TRUE;

           \* unlock
   u0:     got_flag := waiters_flag;
           got_tid := holder_tid;
           waiters_flag := FALSE;
           holder_tid := Null;
           assert in_critical_section;
           in_critical_section := FALSE;
           \* This is the interesting signal since we don't hold the
           \* lock but might be responsible for waking a thread, or
           \* even our own thread!
   u1:     maybe_signal();
   u2:     if (got_flag) {
               futex_wake()
           };

   u_end:  if (iterations > 0) {
             \* Reduce state space by clamping to 0. Otherwise, it can go
             \* negative if a signal handler set it to 0.
             iterations := iterations - 1;
           };
           return;
   }
   fair process (P \in Proc)
     variable iterations=MaxIterations,
              signal_depth=0;
     {
   start: while(iterations > 0) {
            call lock_unlock();
          };
   p_done:skip;
     }
 }
*********)
\* BEGIN TRANSLATION (chksum(pcal) = "cf68c1cf" /\ chksum(tla) = "383b1d4c")
VARIABLES holder_tid, waiters_flag, waiters_set, is_waiting, 
          in_critical_section, pc, stack, got_tid, got_flag, iterations, 
          signal_depth

vars == << holder_tid, waiters_flag, waiters_set, is_waiting, 
           in_critical_section, pc, stack, got_tid, got_flag, iterations, 
           signal_depth >>

ProcSet == (Proc)

Init == (* Global variables *)
        /\ holder_tid = Null
        /\ waiters_flag = FALSE
        /\ waiters_set = {}
        /\ is_waiting = [ i \in Proc |-> FALSE ]
        /\ in_critical_section = FALSE
        (* Procedure lock_unlock *)
        /\ got_tid = [ self \in ProcSet |-> Null]
        /\ got_flag = [ self \in ProcSet |-> FALSE]
        (* Process P *)
        /\ iterations = [self \in Proc |-> MaxIterations]
        /\ signal_depth = [self \in Proc |-> 0]
        /\ stack = [self \in ProcSet |-> << >>]
        /\ pc = [self \in ProcSet |-> "start"]

s0(self) == /\ pc[self] = "s0"
            /\ signal_depth' = [signal_depth EXCEPT ![self] = signal_depth[self] + 1]
            /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "lock_unlock",
                                                     pc        |->  "s1",
                                                     got_tid   |->  got_tid[self],
                                                     got_flag  |->  got_flag[self] ] >>
                                                 \o stack[self]]
            /\ got_tid' = [got_tid EXCEPT ![self] = Null]
            /\ got_flag' = [got_flag EXCEPT ![self] = FALSE]
            /\ pc' = [pc EXCEPT ![self] = "l0"]
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, iterations >>

s1(self) == /\ pc[self] = "s1"
            /\ signal_depth' = [signal_depth EXCEPT ![self] = signal_depth[self] - 1]
            /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
            /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, got_tid, got_flag, iterations >>

handle_signal(self) == s0(self) \/ s1(self)

l0(self) == /\ pc[self] = "l0"
            /\ IF holder_tid = Null /\ waiters_flag = FALSE
                  THEN /\ holder_tid' = self
                       /\ pc' = [pc EXCEPT ![self] = "cs"]
                       /\ UNCHANGED << got_tid, got_flag >>
                  ELSE /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
                       /\ got_flag' = [got_flag EXCEPT ![self] = waiters_flag]
                       /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
                       /\ UNCHANGED holder_tid
            /\ UNCHANGED << waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, stack, iterations, 
                            signal_depth >>

l_cnt(self) == /\ pc[self] = "l_cnt"
               /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                     THEN /\ \/ /\ TRUE
                                /\ pc' = [pc EXCEPT ![self] = "l1"]
                                /\ stack' = stack
                             \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                         pc        |->  "l1" ] >>
                                                                     \o stack[self]]
                                /\ pc' = [pc EXCEPT ![self] = "s0"]
                     ELSE /\ pc' = [pc EXCEPT ![self] = "l1"]
                          /\ stack' = stack
               /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, 
                               is_waiting, in_critical_section, got_tid, 
                               got_flag, iterations, signal_depth >>

l1(self) == /\ pc[self] = "l1"
            /\ IF got_tid[self] = self
                  THEN /\ Assert(signal_depth[self] > 0, 
                                 "Failure of assertion at line 76, column 16.")
                       /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
                       /\ got_tid' = [got_tid EXCEPT ![self] = Head(stack[self]).got_tid]
                       /\ got_flag' = [got_flag EXCEPT ![self] = Head(stack[self]).got_flag]
                       /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l2"]
                       /\ UNCHANGED << stack, got_tid, got_flag >>
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, iterations, signal_depth >>

l2(self) == /\ pc[self] = "l2"
            /\ IF got_tid[self] = Null
                  THEN /\ IF holder_tid = got_tid[self] /\ waiters_flag = got_flag[self]
                             THEN /\ holder_tid' = self
                                  /\ waiters_flag' = TRUE
                                  /\ pc' = [pc EXCEPT ![self] = "cs"]
                                  /\ UNCHANGED << got_tid, got_flag >>
                             ELSE /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
                                  /\ got_flag' = [got_flag EXCEPT ![self] = waiters_flag]
                                  /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
                                  /\ UNCHANGED << holder_tid, waiters_flag >>
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l3"]
                       /\ UNCHANGED << holder_tid, waiters_flag, got_tid, 
                                       got_flag >>
            /\ UNCHANGED << waiters_set, is_waiting, in_critical_section, 
                            stack, iterations, signal_depth >>

l3(self) == /\ pc[self] = "l3"
            /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                  THEN /\ \/ /\ TRUE
                             /\ pc' = [pc EXCEPT ![self] = "l4"]
                             /\ stack' = stack
                          \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                      pc        |->  "l4" ] >>
                                                                  \o stack[self]]
                             /\ pc' = [pc EXCEPT ![self] = "s0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l4"]
                       /\ stack' = stack
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, got_tid, got_flag, iterations, 
                            signal_depth >>

l4(self) == /\ pc[self] = "l4"
            /\ IF got_flag[self] = FALSE
                  THEN /\ IF waiters_flag = got_flag[self] /\ holder_tid = got_tid[self]
                             THEN /\ waiters_flag' = TRUE
                                  /\ got_flag' = [got_flag EXCEPT ![self] = TRUE]
                                  /\ pc' = [pc EXCEPT ![self] = "l5"]
                                  /\ UNCHANGED got_tid
                             ELSE /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
                                  /\ got_flag' = [got_flag EXCEPT ![self] = waiters_flag]
                                  /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
                                  /\ UNCHANGED waiters_flag
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l5"]
                       /\ UNCHANGED << waiters_flag, got_tid, got_flag >>
            /\ UNCHANGED << holder_tid, waiters_set, is_waiting, 
                            in_critical_section, stack, iterations, 
                            signal_depth >>

l5(self) == /\ pc[self] = "l5"
            /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                  THEN /\ \/ /\ TRUE
                             /\ pc' = [pc EXCEPT ![self] = "w0"]
                             /\ stack' = stack
                          \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                      pc        |->  "w0" ] >>
                                                                  \o stack[self]]
                             /\ pc' = [pc EXCEPT ![self] = "s0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "w0"]
                       /\ stack' = stack
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, got_tid, got_flag, iterations, 
                            signal_depth >>

w0(self) == /\ pc[self] = "w0"
            /\ IF holder_tid = got_tid[self] /\ waiters_flag = got_flag[self]
                  THEN /\ waiters_set' = (waiters_set \union { self })
                       /\ is_waiting' = [is_waiting EXCEPT ![self] = TRUE]
                       /\ pc' = [pc EXCEPT ![self] = "w1"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l6"]
                       /\ UNCHANGED << waiters_set, is_waiting >>
            /\ UNCHANGED << holder_tid, waiters_flag, in_critical_section, 
                            stack, got_tid, got_flag, iterations, signal_depth >>

w1(self) == /\ pc[self] = "w1"
            /\ is_waiting[self] = FALSE
            /\ pc' = [pc EXCEPT ![self] = "l6"]
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, stack, got_tid, got_flag, 
                            iterations, signal_depth >>

l6(self) == /\ pc[self] = "l6"
            /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                  THEN /\ \/ /\ TRUE
                             /\ pc' = [pc EXCEPT ![self] = "l7"]
                             /\ stack' = stack
                          \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                      pc        |->  "l7" ] >>
                                                                  \o stack[self]]
                             /\ pc' = [pc EXCEPT ![self] = "s0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l7"]
                       /\ stack' = stack
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, got_tid, got_flag, iterations, 
                            signal_depth >>

l7(self) == /\ pc[self] = "l7"
            /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
            /\ got_flag' = [got_flag EXCEPT ![self] = waiters_flag]
            /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, stack, iterations, 
                            signal_depth >>

cs(self) == /\ pc[self] = "cs"
            /\ Assert(~in_critical_section, 
                      "Failure of assertion at line 117, column 12.")
            /\ in_critical_section' = TRUE
            /\ pc' = [pc EXCEPT ![self] = "u0"]
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            stack, got_tid, got_flag, iterations, signal_depth >>

u0(self) == /\ pc[self] = "u0"
            /\ got_flag' = [got_flag EXCEPT ![self] = waiters_flag]
            /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
            /\ waiters_flag' = FALSE
            /\ holder_tid' = Null
            /\ Assert(in_critical_section, 
                      "Failure of assertion at line 125, column 12.")
            /\ in_critical_section' = FALSE
            /\ pc' = [pc EXCEPT ![self] = "u1"]
            /\ UNCHANGED << waiters_set, is_waiting, stack, iterations, 
                            signal_depth >>

u1(self) == /\ pc[self] = "u1"
            /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                  THEN /\ \/ /\ TRUE
                             /\ pc' = [pc EXCEPT ![self] = "u2"]
                             /\ stack' = stack
                          \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                      pc        |->  "u2" ] >>
                                                                  \o stack[self]]
                             /\ pc' = [pc EXCEPT ![self] = "s0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "u2"]
                       /\ stack' = stack
            /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, is_waiting, 
                            in_critical_section, got_tid, got_flag, iterations, 
                            signal_depth >>

u2(self) == /\ pc[self] = "u2"
            /\ IF got_flag[self]
                  THEN /\ IF waiters_set # {}
                             THEN /\ \E p \in waiters_set:
                                       /\ waiters_set' = waiters_set \ {p}
                                       /\ is_waiting' = [is_waiting EXCEPT ![p] = FALSE]
                             ELSE /\ TRUE
                                  /\ UNCHANGED << waiters_set, is_waiting >>
                  ELSE /\ TRUE
                       /\ UNCHANGED << waiters_set, is_waiting >>
            /\ pc' = [pc EXCEPT ![self] = "u_end"]
            /\ UNCHANGED << holder_tid, waiters_flag, in_critical_section, 
                            stack, got_tid, got_flag, iterations, signal_depth >>

u_end(self) == /\ pc[self] = "u_end"
               /\ IF iterations[self] > 0
                     THEN /\ iterations' = [iterations EXCEPT ![self] = iterations[self] - 1]
                     ELSE /\ TRUE
                          /\ UNCHANGED iterations
               /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
               /\ got_tid' = [got_tid EXCEPT ![self] = Head(stack[self]).got_tid]
               /\ got_flag' = [got_flag EXCEPT ![self] = Head(stack[self]).got_flag]
               /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
               /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, 
                               is_waiting, in_critical_section, signal_depth >>

lock_unlock(self) == l0(self) \/ l_cnt(self) \/ l1(self) \/ l2(self)
                        \/ l3(self) \/ l4(self) \/ l5(self) \/ w0(self)
                        \/ w1(self) \/ l6(self) \/ l7(self) \/ cs(self)
                        \/ u0(self) \/ u1(self) \/ u2(self) \/ u_end(self)

start(self) == /\ pc[self] = "start"
               /\ IF iterations[self] > 0
                     THEN /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "lock_unlock",
                                                                   pc        |->  "start",
                                                                   got_tid   |->  got_tid[self],
                                                                   got_flag  |->  got_flag[self] ] >>
                                                               \o stack[self]]
                          /\ got_tid' = [got_tid EXCEPT ![self] = Null]
                          /\ got_flag' = [got_flag EXCEPT ![self] = FALSE]
                          /\ pc' = [pc EXCEPT ![self] = "l0"]
                     ELSE /\ pc' = [pc EXCEPT ![self] = "p_done"]
                          /\ UNCHANGED << stack, got_tid, got_flag >>
               /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, 
                               is_waiting, in_critical_section, iterations, 
                               signal_depth >>

p_done(self) == /\ pc[self] = "p_done"
                /\ TRUE
                /\ pc' = [pc EXCEPT ![self] = "Done"]
                /\ UNCHANGED << holder_tid, waiters_flag, waiters_set, 
                                is_waiting, in_critical_section, stack, 
                                got_tid, got_flag, iterations, signal_depth >>

P(self) == start(self) \/ p_done(self)

(* Allow infinite stuttering to prevent deadlock on termination. *)
Terminating == /\ \A self \in ProcSet: pc[self] = "Done"
               /\ UNCHANGED vars

Next == (\E self \in ProcSet: handle_signal(self) \/ lock_unlock(self))
           \/ (\E self \in Proc: P(self))
           \/ Terminating

Spec == /\ Init /\ [][Next]_vars
        /\ WF_vars(Next)
        /\ \A self \in Proc : WF_vars(P(self)) /\ WF_vars(lock_unlock(self))

Termination == <>(\A self \in ProcSet: pc[self] = "Done")

\* END TRANSLATION
=============================================================================
\* Modification History
\* Last modified Fri Feb 27 13:21:15 MST 2026 by eric
\* Created Thu Feb 26 15:38:01 MST 2026 by eric

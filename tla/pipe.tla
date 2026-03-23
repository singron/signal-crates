-------------------------------- MODULE pipe --------------------------------

EXTENDS Integers, Sequences, TLC, FiniteSets

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
    pipe_buf_len=0,
    waiters=0,
    waiters_set={};
    in_critical_section = FALSE;
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
     variable got_tid=Null, got_waiters=0, pending_byte=FALSE;
   {
           \* lock
   l0:     if (holder_tid = Null) {
             holder_tid := self;
           } else {
             got_tid := holder_tid;

             \* lock_contended
   l_pre1:   if (got_tid = self) {
               assert signal_depth > 0;
               return; \* Err(Recursive)
             };
   l_pre2:   waiters := waiters+1; pending_byte := TRUE;
   l_pre3:   got_tid := holder_tid;
   l_cnt:    maybe_signal();
   l2:       if (got_tid = Null) {
               if (holder_tid = got_tid) {
                 holder_tid := self;
   l22:          assert waiters > 0;
                 waiters := waiters - 1;
                 goto cs;
               } else {
                 got_tid := holder_tid;
                 goto l_cnt;
               }
             };
   l3:       maybe_signal();

             \* pipe read
   pr0:      await pipe_buf_len > 0;
             pipe_buf_len := pipe_buf_len - 1;

   l6:       maybe_signal();
   l8:       got_tid := holder_tid;
             goto l_cnt;
           };

           \* critical section
           \*   We use a assert on a variable instead of a safety property on
           \*   pc["cs"] since we want a Proc to also be mutally excluded with
           \*   its own signal handlers.
   cs:     assert ~in_critical_section;
           in_critical_section := TRUE;

           \* unlock
   u0:     got_tid := Null;
           holder_tid := Null;
           assert in_critical_section;
           in_critical_section := FALSE;
           \* This is the interesting signal since we don't hold the
           \* lock but might be responsible for waking a thread, or
           \* even our own thread!
   u1:     maybe_signal();
   u2:     if (waiters > 0 \/ pending_byte) {
             pending_byte := FALSE;
             \* pipe write
   pw0:      if (pipe_buf_len < 1) {
               pipe_buf_len := pipe_buf_len + 1;
             };
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
\* BEGIN TRANSLATION (chksum(pcal) = "5f26cc45" /\ chksum(tla) = "8d5c26be")
VARIABLES holder_tid, pipe_buf_len, waiters, waiters_set, in_critical_section, 
          pc, stack, got_tid, got_waiters, pending_byte, iterations, 
          signal_depth

vars == << holder_tid, pipe_buf_len, waiters, waiters_set, 
           in_critical_section, pc, stack, got_tid, got_waiters, pending_byte, 
           iterations, signal_depth >>

ProcSet == (Proc)

Init == (* Global variables *)
        /\ holder_tid = Null
        /\ pipe_buf_len = 0
        /\ waiters = 0
        /\ waiters_set = {}
        /\ in_critical_section = FALSE
        (* Procedure lock_unlock *)
        /\ got_tid = [ self \in ProcSet |-> Null]
        /\ got_waiters = [ self \in ProcSet |-> 0]
        /\ pending_byte = [ self \in ProcSet |-> FALSE]
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
                                                     got_waiters |->  got_waiters[self],
                                                     pending_byte |->  pending_byte[self] ] >>
                                                 \o stack[self]]
            /\ got_tid' = [got_tid EXCEPT ![self] = Null]
            /\ got_waiters' = [got_waiters EXCEPT ![self] = 0]
            /\ pending_byte' = [pending_byte EXCEPT ![self] = FALSE]
            /\ pc' = [pc EXCEPT ![self] = "l0"]
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, iterations >>

s1(self) == /\ pc[self] = "s1"
            /\ signal_depth' = [signal_depth EXCEPT ![self] = signal_depth[self] - 1]
            /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
            /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, got_tid, got_waiters, 
                            pending_byte, iterations >>

handle_signal(self) == s0(self) \/ s1(self)

l0(self) == /\ pc[self] = "l0"
            /\ IF holder_tid = Null
                  THEN /\ holder_tid' = self
                       /\ pc' = [pc EXCEPT ![self] = "cs"]
                       /\ UNCHANGED got_tid
                  ELSE /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
                       /\ pc' = [pc EXCEPT ![self] = "l_pre1"]
                       /\ UNCHANGED holder_tid
            /\ UNCHANGED << pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, stack, got_waiters, 
                            pending_byte, iterations, signal_depth >>

l_pre1(self) == /\ pc[self] = "l_pre1"
                /\ IF got_tid[self] = self
                      THEN /\ Assert(signal_depth[self] > 0, 
                                     "Failure of assertion at line 66, column 16.")
                           /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
                           /\ got_tid' = [got_tid EXCEPT ![self] = Head(stack[self]).got_tid]
                           /\ got_waiters' = [got_waiters EXCEPT ![self] = Head(stack[self]).got_waiters]
                           /\ pending_byte' = [pending_byte EXCEPT ![self] = Head(stack[self]).pending_byte]
                           /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
                      ELSE /\ pc' = [pc EXCEPT ![self] = "l_pre2"]
                           /\ UNCHANGED << stack, got_tid, got_waiters, 
                                           pending_byte >>
                /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                                in_critical_section, iterations, signal_depth >>

l_pre2(self) == /\ pc[self] = "l_pre2"
                /\ waiters' = waiters+1
                /\ pending_byte' = [pending_byte EXCEPT ![self] = TRUE]
                /\ pc' = [pc EXCEPT ![self] = "l_pre3"]
                /\ UNCHANGED << holder_tid, pipe_buf_len, waiters_set, 
                                in_critical_section, stack, got_tid, 
                                got_waiters, iterations, signal_depth >>

l_pre3(self) == /\ pc[self] = "l_pre3"
                /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
                /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
                /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                                in_critical_section, stack, got_waiters, 
                                pending_byte, iterations, signal_depth >>

l_cnt(self) == /\ pc[self] = "l_cnt"
               /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                     THEN /\ \/ /\ TRUE
                                /\ pc' = [pc EXCEPT ![self] = "l2"]
                                /\ stack' = stack
                             \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                         pc        |->  "l2" ] >>
                                                                     \o stack[self]]
                                /\ pc' = [pc EXCEPT ![self] = "s0"]
                     ELSE /\ pc' = [pc EXCEPT ![self] = "l2"]
                          /\ stack' = stack
               /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                               in_critical_section, got_tid, got_waiters, 
                               pending_byte, iterations, signal_depth >>

l2(self) == /\ pc[self] = "l2"
            /\ IF got_tid[self] = Null
                  THEN /\ IF holder_tid = got_tid[self]
                             THEN /\ holder_tid' = self
                                  /\ pc' = [pc EXCEPT ![self] = "l22"]
                                  /\ UNCHANGED got_tid
                             ELSE /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
                                  /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
                                  /\ UNCHANGED holder_tid
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l3"]
                       /\ UNCHANGED << holder_tid, got_tid >>
            /\ UNCHANGED << pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, stack, got_waiters, 
                            pending_byte, iterations, signal_depth >>

l22(self) == /\ pc[self] = "l22"
             /\ Assert(waiters > 0, 
                       "Failure of assertion at line 75, column 18.")
             /\ waiters' = waiters - 1
             /\ pc' = [pc EXCEPT ![self] = "cs"]
             /\ UNCHANGED << holder_tid, pipe_buf_len, waiters_set, 
                             in_critical_section, stack, got_tid, got_waiters, 
                             pending_byte, iterations, signal_depth >>

l3(self) == /\ pc[self] = "l3"
            /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                  THEN /\ \/ /\ TRUE
                             /\ pc' = [pc EXCEPT ![self] = "pr0"]
                             /\ stack' = stack
                          \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                      pc        |->  "pr0" ] >>
                                                                  \o stack[self]]
                             /\ pc' = [pc EXCEPT ![self] = "s0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "pr0"]
                       /\ stack' = stack
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, got_tid, got_waiters, 
                            pending_byte, iterations, signal_depth >>

pr0(self) == /\ pc[self] = "pr0"
             /\ pipe_buf_len > 0
             /\ pipe_buf_len' = pipe_buf_len - 1
             /\ pc' = [pc EXCEPT ![self] = "l6"]
             /\ UNCHANGED << holder_tid, waiters, waiters_set, 
                             in_critical_section, stack, got_tid, got_waiters, 
                             pending_byte, iterations, signal_depth >>

l6(self) == /\ pc[self] = "l6"
            /\ IF iterations[self] > 0 /\ signal_depth[self] < MaxSignalDepth
                  THEN /\ \/ /\ TRUE
                             /\ pc' = [pc EXCEPT ![self] = "l8"]
                             /\ stack' = stack
                          \/ /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "handle_signal",
                                                                      pc        |->  "l8" ] >>
                                                                  \o stack[self]]
                             /\ pc' = [pc EXCEPT ![self] = "s0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "l8"]
                       /\ stack' = stack
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, got_tid, got_waiters, 
                            pending_byte, iterations, signal_depth >>

l8(self) == /\ pc[self] = "l8"
            /\ got_tid' = [got_tid EXCEPT ![self] = holder_tid]
            /\ pc' = [pc EXCEPT ![self] = "l_cnt"]
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, stack, got_waiters, 
                            pending_byte, iterations, signal_depth >>

cs(self) == /\ pc[self] = "cs"
            /\ Assert(~in_critical_section, 
                      "Failure of assertion at line 98, column 12.")
            /\ in_critical_section' = TRUE
            /\ pc' = [pc EXCEPT ![self] = "u0"]
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            stack, got_tid, got_waiters, pending_byte, 
                            iterations, signal_depth >>

u0(self) == /\ pc[self] = "u0"
            /\ got_tid' = [got_tid EXCEPT ![self] = Null]
            /\ holder_tid' = Null
            /\ Assert(in_critical_section, 
                      "Failure of assertion at line 104, column 12.")
            /\ in_critical_section' = FALSE
            /\ pc' = [pc EXCEPT ![self] = "u1"]
            /\ UNCHANGED << pipe_buf_len, waiters, waiters_set, stack, 
                            got_waiters, pending_byte, iterations, 
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
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, got_tid, got_waiters, 
                            pending_byte, iterations, signal_depth >>

u2(self) == /\ pc[self] = "u2"
            /\ IF waiters > 0 \/ pending_byte[self]
                  THEN /\ pending_byte' = [pending_byte EXCEPT ![self] = FALSE]
                       /\ pc' = [pc EXCEPT ![self] = "pw0"]
                  ELSE /\ pc' = [pc EXCEPT ![self] = "u_end"]
                       /\ UNCHANGED pending_byte
            /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                            in_critical_section, stack, got_tid, got_waiters, 
                            iterations, signal_depth >>

pw0(self) == /\ pc[self] = "pw0"
             /\ IF pipe_buf_len < 1
                   THEN /\ pipe_buf_len' = pipe_buf_len + 1
                   ELSE /\ TRUE
                        /\ UNCHANGED pipe_buf_len
             /\ pc' = [pc EXCEPT ![self] = "u_end"]
             /\ UNCHANGED << holder_tid, waiters, waiters_set, 
                             in_critical_section, stack, got_tid, got_waiters, 
                             pending_byte, iterations, signal_depth >>

u_end(self) == /\ pc[self] = "u_end"
               /\ IF iterations[self] > 0
                     THEN /\ iterations' = [iterations EXCEPT ![self] = iterations[self] - 1]
                     ELSE /\ TRUE
                          /\ UNCHANGED iterations
               /\ pc' = [pc EXCEPT ![self] = Head(stack[self]).pc]
               /\ got_tid' = [got_tid EXCEPT ![self] = Head(stack[self]).got_tid]
               /\ got_waiters' = [got_waiters EXCEPT ![self] = Head(stack[self]).got_waiters]
               /\ pending_byte' = [pending_byte EXCEPT ![self] = Head(stack[self]).pending_byte]
               /\ stack' = [stack EXCEPT ![self] = Tail(stack[self])]
               /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                               in_critical_section, signal_depth >>

lock_unlock(self) == l0(self) \/ l_pre1(self) \/ l_pre2(self)
                        \/ l_pre3(self) \/ l_cnt(self) \/ l2(self)
                        \/ l22(self) \/ l3(self) \/ pr0(self) \/ l6(self)
                        \/ l8(self) \/ cs(self) \/ u0(self) \/ u1(self)
                        \/ u2(self) \/ pw0(self) \/ u_end(self)

start(self) == /\ pc[self] = "start"
               /\ IF iterations[self] > 0
                     THEN /\ stack' = [stack EXCEPT ![self] = << [ procedure |->  "lock_unlock",
                                                                   pc        |->  "start",
                                                                   got_tid   |->  got_tid[self],
                                                                   got_waiters |->  got_waiters[self],
                                                                   pending_byte |->  pending_byte[self] ] >>
                                                               \o stack[self]]
                          /\ got_tid' = [got_tid EXCEPT ![self] = Null]
                          /\ got_waiters' = [got_waiters EXCEPT ![self] = 0]
                          /\ pending_byte' = [pending_byte EXCEPT ![self] = FALSE]
                          /\ pc' = [pc EXCEPT ![self] = "l0"]
                     ELSE /\ pc' = [pc EXCEPT ![self] = "p_done"]
                          /\ UNCHANGED << stack, got_tid, got_waiters, 
                                          pending_byte >>
               /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                               in_critical_section, iterations, signal_depth >>

p_done(self) == /\ pc[self] = "p_done"
                /\ TRUE
                /\ pc' = [pc EXCEPT ![self] = "Done"]
                /\ UNCHANGED << holder_tid, pipe_buf_len, waiters, waiters_set, 
                                in_critical_section, stack, got_tid, 
                                got_waiters, pending_byte, iterations, 
                                signal_depth >>

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

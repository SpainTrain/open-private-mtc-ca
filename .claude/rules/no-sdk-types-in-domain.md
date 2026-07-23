# no-sdk-types-in-domain

> Spec: §22.8 (Repository pattern boundary — no SDK types in domain code).

## Rule

Vendor SDK types (`aws_sdk_*`, etc.) must never appear in trait signatures or
domain code. Translation between domain types and SDK types happens inside the
backend implementation crate, and only there.

## Rationale

§22.8: if a trait signature mentions `aws_sdk_*`, the trait is wrong. The cloud
abstraction traits (§9) take and return domain types only. An agent that sees
an SDK type leak through one trait will repeat the pattern, coupling the CA
service's domain logic to AWS and defeating the cloud abstraction. The rule
applies in both directions: AWS-implementation code accepts domain types and
emits SDK calls; domain types never reach the SDK directly. The translation
layer stays small and lives in the backend crate.

## Compliant example

```rust
// domain trait — domain types only (§22.8)
#[async_trait]
pub trait ReplicatedKv: Send + Sync {
    async fn atomic_update(
        &self,
        key: &Key,               // domain type
        expr: UpdateExpression,  // domain type
        conditions: &[Condition],
    ) -> Result<Item, Error>;
}
// The AWS impl crate converts Condition -> aws_sdk_dynamodb::types::AttributeValue
// internally, never exposing the SDK type outward.
```

## Non-compliant example

```rust
// SDK types leak through the trait boundary
async fn atomic_update(
    &self,
    input: aws_sdk_dynamodb::operation::update_item::UpdateItemInput,
) -> Result<aws_sdk_dynamodb::operation::update_item::UpdateItemOutput, _>;
```

## Enforcement

- **Lint / CI gate**: domain crates do not depend on `aws_sdk_*` in
  `Cargo.toml`, so a leak fails to compile; `cargo deny check` (§22.13) keeps
  the dependency graph honest.
- **Review**: any trait signature or domain struct mentioning a vendor SDK
  type is rejected outright.
